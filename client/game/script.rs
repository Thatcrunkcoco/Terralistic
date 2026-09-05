/// A parsed, self-contained script driver for the `--script <file>`
/// scripted-play harness. Actions are sequenced by accumulating `wait`
/// delays; key holds and chat typing schedule their companion release /
/// follow-up events onto an internal timed queue.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};

use crate::libraries::graphics as gfx;

const CHAT_OPEN_DELAY_MS: u64 = 60;
const CHAT_CHAR_DELAY_MS: u64 = 15;
const CHAT_ENTER_DELAY_MS: u64 = 60;
const CHAT_ESC_DELAY_MS: u64 = 40;

/// What the driver wants the game loop to do this frame.
#[derive(Default)]
pub struct ScriptStatus {
    pub shot: bool,
    pub exit: bool,
    /// client-side teleport of the main player (server /tp is undone by the
    /// client's own authoritative position reports)
    pub tp: Option<(f32, f32)>,
    /// world target in *screen mouse units* (already resolved); the game loop
    /// verifies the selected block matches what the script wanted
    pub world_click: Option<WorldClickSpec>,
}

/// A world click resolved by the script: click the mouse at the screen
/// position of world block `(block_x, block_y)` (held `hold_ms`).
#[derive(Debug, Clone)]
pub struct WorldClickSpec {
    pub block_x: i32,
    pub block_y: i32,
    pub button: gfx::Key,
    pub hold_ms: u64,
    pub mouse_x: f32,
    pub mouse_y: f32,
}

/// One scripted action. Key/button names follow `parse_key` / `parse_button`;
/// `world_click` is resolved against the live camera at fire time, so it is
/// stored symbolically.
#[derive(Debug, Clone)]
pub enum ScriptAction {
    Wait { ms: u64 },
    Press { key: gfx::Key, hold_ms: u64 },
    Move { x: f32, y: f32 },
    /// Send `text` as a chat message (`T` + text + `Enter`). A leading `/`
    /// makes it a server command.
    Chat { text: String },
    /// Teleport the local player to world position `(x, y)` (block coords on
    /// the flat test world grid, in world units not blocks).
    Tp { x: f32, y: f32 },
    /// Left/right click at the center of world block `(x, y)`.
    WorldClick { block_x: i32, block_y: i32, button: gfx::Key, hold_ms: u64 },
    /// Force a capture on the next frame.
    Shot,
    Exit,
}

/// One event that should be injected `due_ms` after the driver started, or a
/// marker clearing the scripted mouse override.
enum PendingEvent {
    Injected { due_ms: u64, event: gfx::Event },
    MouseClear { due_ms: u64 },
}

pub struct ScriptDriver {
    actions: Vec<ScriptAction>,
    /// cumulative time (since script start) when each action becomes due
    actions_ms: Vec<u64>,
    next_index: usize,
    start: Instant,
    pending: VecDeque<PendingEvent>,
    /// The current scripted mouse pointer (in scaled UI units, exactly like
    /// the real accessor); `None` once the mouse is released back.
    mouse_pos: Option<gfx::FloatPos>,
}

impl ScriptDriver {
    /// Parses a script file. One action per line; `#` comments and blank
    /// lines are skipped.
    pub fn from_file(path: &str) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let mut actions = Vec::new();
        let mut actions_ms = Vec::new();
        let mut time = 0u64;
        for line in contents.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let action = parse_line(line)?;
            time += action.duration_ms();
            actions.push(action);
            actions_ms.push(time);
        }
        Ok(Self {
            actions,
            actions_ms,
            next_index: 0,
            start: Instant::now(),
            pending: VecDeque::new(),
            mouse_pos: None,
        })
    }

    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn due_ms(&self, index: usize) -> u64 {
        self.actions_ms[index]
    }

    fn inject_at(&mut self, delay_ms: u64, event: gfx::Event) {
        self.pending.push_back(PendingEvent::Injected { due_ms: self.now_ms() + delay_ms, event });
    }

    /// Pumps the driver once per rendered frame, before the normal event
    /// drain, so injected events land in the same frame.
    pub fn update(&mut self, graphics: &mut gfx::GraphicsContext, camera_x: f32, camera_y: f32) -> Result<ScriptStatus> {
        let mut status = ScriptStatus::default();
        let now = self.now_ms();

        // keep the scripted mouse override in sync
        graphics.set_scripted_mouse_pos(self.mouse_pos);

        // release any due timed events
        loop {
            let Some(pending) = self.pending.front() else { break };
            let due_ms = match pending {
                PendingEvent::Injected { due_ms, .. } | PendingEvent::MouseClear { due_ms } => *due_ms,
            };
            if due_ms > now {
                break;
            }
            let taken = self.pending.pop_front().unwrap_or_else(|| panic!("front was Some; checked above"));
            match taken {
                PendingEvent::Injected { event, .. } => graphics.inject_event(event),
                PendingEvent::MouseClear { .. } => {
                    self.mouse_pos = None;
                    graphics.set_scripted_mouse_pos(None);
                }
            }
        }

        // fire all actions whose time has come
        while self.next_index < self.actions.len() && self.due_ms(self.next_index) <= now {
            match self.actions[self.next_index].clone() {
                ScriptAction::Wait { ms } => {
                    // the schedule already accounts for this delay; sleeping
                    // here and shifting the start time keeps the cadence of
                    // all subsequent actions grid-aligned to real action time
                    std::thread::sleep(Duration::from_millis(ms));
                    self.start += Duration::from_millis(ms);
                }
                ScriptAction::Press { key, hold_ms } => {
                    graphics.inject_event(gfx::Event::KeyPress(key, false));
                    self.inject_at(hold_ms + 1, gfx::Event::KeyRelease(key, false));
                }
                ScriptAction::Move { x, y } => {
                    self.mouse_pos = Some(gfx::FloatPos(x, y));
                    graphics.set_scripted_mouse_pos(Some(gfx::FloatPos(x, y)));
                }
                ScriptAction::Chat { text } => {
                    graphics.inject_event(gfx::Event::KeyPress(gfx::Key::T, false));
                    self.inject_at(CHAT_OPEN_DELAY_MS, gfx::Event::KeyRelease(gfx::Key::T, false));
                    // Chat has an anti-echo guard that swallows the first
                    // TextInput after opening (the SDL text echo of the
                    // physical `T` key). Mirror real typing: send an empty
                    // TextInput first (eats the guard, adds nothing to the
                    // field), then the actual text, then Enter.
                    self.inject_at(CHAT_OPEN_DELAY_MS + CHAT_CHAR_DELAY_MS, gfx::Event::TextInput(String::new()));
                    self.inject_at(CHAT_OPEN_DELAY_MS + 2 * CHAT_CHAR_DELAY_MS, gfx::Event::TextInput(text));
                    self.inject_at(CHAT_OPEN_DELAY_MS + 2 * CHAT_CHAR_DELAY_MS + CHAT_ENTER_DELAY_MS, gfx::Event::KeyPress(gfx::Key::Enter, false));
                    self.inject_at(CHAT_OPEN_DELAY_MS + 2 * CHAT_CHAR_DELAY_MS + CHAT_ENTER_DELAY_MS + 1, gfx::Event::KeyRelease(gfx::Key::Enter, false));
                    // the chat box stays open after sending; close it with
                    // Escape so it stops consuming all later input events
                    self.inject_at(CHAT_OPEN_DELAY_MS + 2 * CHAT_CHAR_DELAY_MS + CHAT_ENTER_DELAY_MS + CHAT_ESC_DELAY_MS, gfx::Event::KeyPress(gfx::Key::Escape, false));
                    self.inject_at(CHAT_OPEN_DELAY_MS + 2 * CHAT_CHAR_DELAY_MS + CHAT_ENTER_DELAY_MS + CHAT_ESC_DELAY_MS + 1, gfx::Event::KeyRelease(gfx::Key::Escape, false));
                    status.shot = true;
                }
                ScriptAction::WorldClick { block_x, block_y, button, hold_ms } => {
                    // convert world block coords to scaled mouse coords using
                    // the camera position captured on this frame
                    let render_block_width = crate::shared::blocks::RENDER_BLOCK_WIDTH;
                    let top_left_x = camera_x - (graphics.get_window_size().0 / render_block_width) / 2.0;
                    let top_left_y = camera_y - (graphics.get_window_size().1 / render_block_width) / 2.0;
                    let mouse_x = (block_x as f32 + 0.5 - top_left_x) * render_block_width;
                    let mouse_y = (block_y as f32 + 0.5 - top_left_y) * render_block_width;
                    self.mouse_pos = Some(gfx::FloatPos(mouse_x, mouse_y));
                    graphics.set_scripted_mouse_pos(Some(gfx::FloatPos(mouse_x, mouse_y)));
                    // park the override on the block; the actual press /
                    // release injection is done by the game loop's steering
                    // step (see run_game world_click_steering), which corrects
                    // for any world<->screen unit drift before pressing
                    self.pending.push_back(PendingEvent::MouseClear { due_ms: self.now_ms() + hold_ms + 400 });
                    status.world_click = Some(WorldClickSpec { block_x, block_y, button, hold_ms, mouse_x, mouse_y });
                    status.shot = true;
                }
                ScriptAction::Tp { x, y } => status.tp = Some((x, y)),
                ScriptAction::Shot => status.shot = true,
                ScriptAction::Exit => status.exit = true,
            }
            self.next_index += 1;
        }

        Ok(status)
    }

    /// Whether the script still has work queued (used to stop the run when
    /// the driver is exhausted; with `--capture` the game exits on its own).
    pub fn is_finished(&self) -> bool {
        self.next_index >= self.actions.len() && self.pending.is_empty()
    }
}

impl ScriptAction {
    /// How much script-time (since script start) this action consumes.
    fn duration_ms(&self) -> u64 {
        match self {
            ScriptAction::Wait { ms } => *ms,
            ScriptAction::Press { hold_ms, .. } | ScriptAction::WorldClick { hold_ms, .. } => hold_ms + 30,
            _ => 30,
        }
    }
}

/// Parses one script line into an action.
fn parse_line(line: &str) -> Result<ScriptAction> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let keyword = *parts.first().unwrap_or(&"");
    let number = |index: usize, what: &str| -> Result<f32> {
        parts
            .get(index)
            .map(|v| v.parse::<f32>())
            .transpose()?
            .ok_or_else(|| anyhow!("\"{}\" needs a number as argument {} (line: {})", keyword, what, line))
    };
    let text_after = |keyword: &str| -> String {
        line.find(keyword).map_or_else(String::new, |i| line[i + keyword.len()..].trim_start().to_owned())
    };
    match keyword {
        "wait" => Ok(ScriptAction::Wait { ms: number(1, "delay")? as u64 }),
        "press" | "hold" => {
            let key = parse_key(parts.get(1).copied().ok_or_else(|| anyhow!("\"{}\" needs a key name (line: {})", keyword, line))?)?;
            let hold_ms = if parts.len() > 2 { number(2, "hold")? as u64 } else { 30 };
            Ok(ScriptAction::Press { key, hold_ms })
        }
        "move" => Ok(ScriptAction::Move { x: number(1, "x")?, y: number(2, "y")? }),
        "tp" => Ok(ScriptAction::Tp { x: number(1, "x")?, y: number(2, "y")? }),
        "chat" => Ok(ScriptAction::Chat { text: text_after("chat") }),
        "worldclick" | "click" => {
            let button_index = if keyword == "worldclick" { 3 } else { 1 };
            let button = parse_button(parts.get(button_index).copied().ok_or_else(|| anyhow!("\"{}\" needs left/right (line: {})", keyword, line))?)?;
            let hold_ms = if parts.len() > button_index + 1 { number(button_index + 1, "hold")? as u64 } else { 30 };
            if keyword == "worldclick" {
                Ok(ScriptAction::WorldClick {
                    block_x: number(1, "block x")? as i32,
                    block_y: number(2, "block y")? as i32,
                    button,
                    hold_ms,
                })
            } else {
                Ok(ScriptAction::Press { key: button, hold_ms })
            }
        }
        "shot" => Ok(ScriptAction::Shot),
        "exit" => Ok(ScriptAction::Exit),
        other => bail!("unknown action \"{}\" (line: {})", other, line),
    }
}

fn parse_key(name: &str) -> Result<gfx::Key> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "space" => Ok(gfx::Key::Space),
        "a" | "b" | "c" | "d" | "e" | "f" | "g" | "h" | "i" | "j" | "k" | "l" | "m" | "n" | "o" | "p" | "q" | "r" | "s" | "t" | "u" | "v" | "w" | "x" | "y" | "z" => {
            match name.as_str() {
                "a" => Ok(gfx::Key::A),
                "b" => Ok(gfx::Key::B),
                "c" => Ok(gfx::Key::C),
                "d" => Ok(gfx::Key::D),
                "e" => Ok(gfx::Key::E),
                "f" => Ok(gfx::Key::F),
                "g" => Ok(gfx::Key::G),
                "h" => Ok(gfx::Key::H),
                "i" => Ok(gfx::Key::I),
                "j" => Ok(gfx::Key::J),
                "k" => Ok(gfx::Key::K),
                "l" => Ok(gfx::Key::L),
                "m" => Ok(gfx::Key::M),
                "n" => Ok(gfx::Key::N),
                "o" => Ok(gfx::Key::O),
                "p" => Ok(gfx::Key::P),
                "q" => Ok(gfx::Key::Q),
                "r" => Ok(gfx::Key::R),
                "s" => Ok(gfx::Key::S),
                "t" => Ok(gfx::Key::T),
                "u" => Ok(gfx::Key::U),
                "v" => Ok(gfx::Key::V),
                "w" => Ok(gfx::Key::W),
                "x" => Ok(gfx::Key::X),
                "y" => Ok(gfx::Key::Y),
                _ => Ok(gfx::Key::Z),
            }
        }
        "0" => Ok(gfx::Key::Num0),
        "1" => Ok(gfx::Key::Num1),
        "2" => Ok(gfx::Key::Num2),
        "3" => Ok(gfx::Key::Num3),
        "4" => Ok(gfx::Key::Num4),
        "5" => Ok(gfx::Key::Num5),
        "6" => Ok(gfx::Key::Num6),
        "7" => Ok(gfx::Key::Num7),
        "8" => Ok(gfx::Key::Num8),
        "9" => Ok(gfx::Key::Num9),
        "escape" | "esc" => Ok(gfx::Key::Escape),
        "enter" | "return" => Ok(gfx::Key::Enter),
        "tab" => Ok(gfx::Key::Tab),
        "backspace" => Ok(gfx::Key::Backspace),
        "delete" | "del" => Ok(gfx::Key::Delete),
        "arrowleft" | "left" => Ok(gfx::Key::Left),
        "arrowright" => Ok(gfx::Key::Right),
        "arrowup" => Ok(gfx::Key::Up),
        "arrowdown" => Ok(gfx::Key::Down),
        "leftshift" | "lshift" | "shift" => Ok(gfx::Key::LeftShift),
        "leftcontrol" | "lctrl" | "ctrl" => Ok(gfx::Key::LeftControl),
        "leftalt" | "lalt" | "alt" => Ok(gfx::Key::LeftAlt),
        "leftmouse" | "mouseleft" | "leftclick" => Ok(gfx::Key::MouseLeft),
        "rightmouse" | "mouseright" | "rightclick" => Ok(gfx::Key::MouseRight),
        "middlemouse" | "mousemiddle" => Ok(gfx::Key::MouseMiddle),
        other => bail!("unknown key \"{}\"", other),
    }
}

fn parse_button(name: &str) -> Result<gfx::Key> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Ok(gfx::Key::MouseLeft),
        "right" => Ok(gfx::Key::MouseRight),
        "middle" => Ok(gfx::Key::MouseMiddle),
        other => bail!("unknown mouse button \"{}\"", other),
    }
}
