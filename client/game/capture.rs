//! Client harness: the `--test --capture <dir>` capture run. Spawns the
//! in-process singleplayer server against a *harness-owned* test world
//! (never touching real singleplayer or server saves), then drives `run_game`
//! with optional scripted input and frame capture.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use anyhow::Result;

use super::core_client::RunGameExtras;
use super::private_world::PrivateWorld;
use crate::client::menus::{Menu, MenuStack};
use crate::libraries::graphics as gfx;
use crate::libraries::graphics::BaseUiElement;

/// Configuration for a single capture run, parsed from the CLI flags.
#[derive(Debug, Default, Clone)]
pub struct CaptureFlags {
    pub capture_dir: Option<String>,
    pub interval: u32,
    pub max_frames: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub script_path: Option<String>,
}

/// Writes PNGs `frame_NNNNN.png` into `capture_dir`, on a cadence of every
/// `interval` rendered frames (or forced on by the script `shot` action).
pub struct FrameCapture {
    dir: std::path::PathBuf,
    interval: u32,
    max_frames: Option<u32>,
    taken: u32,
    frames_seen: u32,
}

impl FrameCapture {
    #[must_use]
    pub fn new(dir: &str, interval: u32, max_frames: Option<u32>) -> Self {
        let _ = std::fs::create_dir_all(dir);
        Self { dir: std::path::PathBuf::from(dir), interval, max_frames, taken: 0, frames_seen: 0 }
    }

    /// Writes the current frame if the cadence says so. `interval == 0`
    /// disables cadence-based capture (the script's `shot` still captures).
    pub fn try_capture(&mut self, graphics: &gfx::GraphicsContext, force: bool) -> Result<bool> {
        if self.taken >= self.max_frames.unwrap_or(u32::MAX) {
            return Ok(false);
        }
        self.frames_seen += 1;
        let due = force || (self.interval > 0 && self.frames_seen % self.interval == 0);
        if !due {
            return Ok(false);
        }
        let path = self.dir.join(format!("frame_{:05}.png", self.taken));
        graphics.capture_frame_to_png(&path)?;
        self.taken += 1;
        Ok(true)
    }    /// Writes a small manifest describing the run.
    pub fn write_manifest(&self) -> Result<()> {
        let manifest = format!(
            "#capture v1\nframes: {}\nframes_seen: {}\ninterval: {}\n",
            self.taken, self.frames_seen, self.interval
        );
        std::fs::write(self.dir.join("manifest.txt"), manifest.as_bytes())?;
        Ok(())
    }

    pub fn is_done(&self) -> bool {
        self.max_frames.map_or(false, |max| self.taken >= max)
    }

    pub const fn frames_taken(&self) -> u32 {
        self.taken
    }
}

/// The whole capture run: fresh harness test world -> the same
/// menu-machinery-driven loading/run/shutdown path a private world takes,
/// with the capture/script extras threaded through into `run_game`.
pub fn run_capture_client(
    graphics: &mut gfx::GraphicsContext,
    flags: CaptureFlags,
    settings: Rc<RefCell<crate::client::settings::Settings>>,
    global_settings: Rc<RefCell<crate::client::global_settings::GlobalSettings>>,
    world_path: &Path,
    debug: bool,
) -> Result<()> {
    // The capture world is deleted before every run so the player always
    // spawns at the default position and terrain is fresh.
    if world_path.exists() {
        std::fs::remove_file(world_path)?;
    }

    // PrivateWorld::new spawns the in-process server itself; do NOT spawn a
    // second one here (a duplicate listener would race the port and save a
    // pristine world over the played one at shutdown).
    let script_path = flags.script_path.clone();
    let capture_dir = flags.capture_dir.clone();
    let capture_interval = flags.interval;
    let capture_max_frames = flags.max_frames;
    let capture_timeout_ms = flags.timeout_ms;
    let world = PrivateWorld::new(
        world_path,
        123,
        "test".to_owned(),
        Rc::clone(&settings),
        Rc::clone(&global_settings),
        debug,
        RunGameExtras {
            script_path,
            capture_dir,
            capture_interval,
            capture_max_frames,
            capture_timeout_ms,
        },
    )?;
    let mut stack = MenuStack::new();
    stack.add_menu((Box::new(world), "CaptureWorld".to_owned()));

    // NOTE: the loop must keep running until the menu stack is empty — the
    // PrivateWorld shutdown step (stopping + joining the server thread, which
    // is where the world save happens) happens inside stack updates after the
    // game has already closed the window.
    let start = std::time::Instant::now();
    let menu_container = gfx::Container::default(graphics);
    while graphics.is_window_open() || !stack.should_close() {
        while let Some(event) = graphics.get_event() {
            // real user input during harness load screens is not needed; drain
            std::mem::drop(event);
        }
        if stack.should_close() {
            graphics.close_window();
        }
        stack.update(graphics, &menu_container);
        if graphics.is_window_open() {
            stack.render(graphics, &menu_container);
        }

        if let Some(timeout) = flags.timeout_ms {
            if start.elapsed().as_millis() as u64 >= timeout {
                graphics.close_window();
            }
        }

        if graphics.is_window_open() {
            graphics.update_window();
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    Ok(())
}
