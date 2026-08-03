use crate::client::game::camera::Camera;
use crate::client::game::gases::ClientGases;
use crate::client::game::networking::ClientNetworking;
use crate::libraries::events::Event;
use crate::libraries::graphics as gfx;
use crate::shared::blocks::RENDER_BLOCK_WIDTH;
use gfx::BaseUiElement;

/// A debug-only overlay that paints every visible gas cell as a semi-transparent
/// rectangle colored by gas type and brightened by pressure. It is meant for
/// vacuum test worlds so the flow simulation (`pressure_rate` /
/// `buoyancy_rate`) can be tuned by eye.
///
/// It can be toggled with the G key, or (in debug mode only) by clicking the
/// "Gases" icon shown in the corner. Toggling it on requests live gas layer
/// updates from the server, which is the only time the server pushes snapshots.
pub struct GasDebugOverlay {
    open: bool,
    debug: bool,
    toggle_button: gfx::Button,
    label_state: bool,
    /// Tracks whether the mouse button went down over the toggle icon. The icon
    /// only toggles on a genuine press-then-release click, so an unrelated mouse
    /// release (e.g. the player letting go of a block-mining hold) never flips it.
    /// A single click is exactly one press + one release, so this needs no
    /// time-based debounce -- rapid clicking must toggle on every click, just
    /// like the G hotkey does.
    mouse_down_on_button: bool,
}

impl GasDebugOverlay {
    #[must_use]
    pub fn new(debug: bool) -> Self {
        let mut toggle_button = gfx::Button::new(|| {});
        toggle_button.scale = 2.0;
        toggle_button.pos.0 = -gfx::SPACING;
        toggle_button.pos.1 = -gfx::SPACING;
        toggle_button.orientation = gfx::TOP_RIGHT;
        Self {
            open: false,
            debug,
            toggle_button,
            label_state: false,
            mouse_down_on_button: false,
        }
    }

    /// Initializes the toggle icon's label. Needs the graphics context for the font.
    pub fn init(&mut self, graphics: &gfx::GraphicsContext) {
        self.toggle_button.texture = gfx::Texture::load_from_surface(&graphics.font.create_text_surface("Gases: OFF", None));
    }

    /// Handles input for the overlay: the G hotkey and the on-screen toggle icon.
    ///
    /// Returns `Ok(true)` when this event was *consumed* by the UI: i.e. a mouse
    /// up/down landed on the toggle icon. The caller should then skip passing the
    /// event to world handlers (e.g. `block_selector`) -- otherwise clicking the
    /// icon in the screen corner would also mine/place the block under the cursor
    /// (or, when the cursor is off-world, crash the server on an out-of-bounds).
    pub fn on_event(
        &mut self,
        event: &Event,
        graphics: &mut gfx::GraphicsContext,
        gases: &mut ClientGases,
        networking: &mut ClientNetworking,
    ) -> Result<bool, anyhow::Error> {
        if let Some(gfx_event) = event.downcast::<gfx::Event>() {
            // trace every mouse/keyboard input so we can see exactly what reaches
            // the overlay when toggling by button vs. by G hotkey.
            if let gfx::Event::KeyPress(gfx::Key::G, false) = gfx_event {
                tracing::debug!("gas_overlay: G hotkey press -> toggle");
                self.toggle(gases, networking)?;
                return Ok(false);
            }
            if self.debug {
                // clicking the icon toggles the overlay (and consumes the event)
                if self.button_click(graphics, gfx_event, gases, networking)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Handles press/release of `MouseLeft`, returning `Ok(true)` when the event
    /// landed on the toggle icon (so the caller knows the UI consumed it and
    /// should not forward it to world handlers).
    ///
    /// This tracks a genuine press-then-release click instead of using the generic
    /// [`gfx::Button`] hook. That hook fires on *any* mouse release while hovered,
    /// even if the button was never pressed down there -- which makes the on-screen
    /// icon unreliable compared to the G hotkey: a player who holds `MouseLeft` to
    /// mine and lets go over the icon would flip it by accident, and rapid clicking
    /// appears to "break" for the same reason. Tracking the press explicitly makes
    /// the icon only toggle on a real click, once per click, exactly matching the G
    /// key (which needs no debounce: one press + one release is one click).
    fn button_click(
        &mut self,
        graphics: &mut gfx::GraphicsContext,
        gfx_event: &gfx::Event,
        gases: &mut ClientGases,
        networking: &mut ClientNetworking,
    ) -> Result<bool, anyhow::Error> {
        match gfx_event {
            gfx::Event::KeyPress(gfx::Key::MouseLeft, _) => {
                let hovered = self.toggle_button.is_hovered(graphics, &gfx::Container::default(graphics));
                tracing::debug!("gas_overlay: ButtonLeft press hovered={hovered}");
                if hovered {
                    self.mouse_down_on_button = true;
                }
                Ok(hovered)
            }
            gfx::Event::KeyRelease(gfx::Key::MouseLeft, _) => {
                let was_down = self.mouse_down_on_button;
                self.mouse_down_on_button = false;
                let hovered = self.toggle_button.is_hovered(graphics, &gfx::Container::default(graphics));
                tracing::debug!("gas_overlay: ButtonLeft release was_down={was_down} hovered={hovered}");
                if was_down && hovered {
                    self.toggle(gases, networking)?;
                }
                // Consume the release if a press began on the icon (even if the
                // release ends just off it), so the icon drag never leaks a
                // world interaction.
                Ok(was_down)
            }
            _ => Ok(false),
        }
    }

    /// Toggles the overlay on/off, starting/stopping live server updates.
    pub fn toggle(&mut self, gases: &mut ClientGases, networking: &mut ClientNetworking) -> Result<(), anyhow::Error> {
        self.open = !self.open;
        tracing::debug!("gas_overlay: toggle -> open={} (requesting live updates)", self.open);
        gases.request_live_updates(self.open, networking)?;
        Ok(())
    }

    /// Renders the overlay (gas cells) plus, in debug mode, the on-screen icon.
    pub fn render(&mut self, graphics: &mut gfx::GraphicsContext, gases: &ClientGases, camera: &Camera) -> Result<(), anyhow::Error> {
        if self.open {
            self.render_gas_cells(graphics, gases, camera)?;
        }

        if self.debug {
            // reflect current state on the icon label (only rebuild when it changes)
            if self.open != self.label_state {
                let label = if self.open { "Gases: ON" } else { "Gases: OFF" };
                self.toggle_button.texture = gfx::Texture::load_from_surface(&graphics.font.create_text_surface(label, None));
                self.label_state = self.open;
            }
            self.toggle_button.render(graphics, &gfx::Container::default(graphics));
        }

        Ok(())
    }

    fn render_gas_cells(&self, graphics: &mut gfx::GraphicsContext, gases: &ClientGases, camera: &Camera) -> Result<(), anyhow::Error> {
        let layer = gases.layer();
        let (width, height) = layer.get_size();
        if width == 0 || height == 0 {
            return Ok(());
        }

        let (top_left_x, top_left_y) = camera.get_top_left(graphics);
        let (bottom_right_x, bottom_right_y) = camera.get_bottom_right(graphics);

        let start_x = i32::max(0, top_left_x as i32);
        let start_y = i32::max(0, top_left_y as i32);
        let end_x = i32::min(width as i32, bottom_right_x as i32 + 1);
        let end_y = i32::min(height as i32, bottom_right_y as i32 + 1);

        // cap the number of cells we render per frame so we never blow up on a
        // huge zoomed-out view
        let max_cells = 120_000;
        let mut drawn = 0usize;

        let mut rect_array = gfx::RectArray::new();
        'outer: for x in start_x..end_x {
            for y in start_y..end_y {
                let cell = layer.get_cell(x, y)?;
                let gas = cell.gas;
                if gas.is_none() {
                    continue;
                }
                let pressure = cell.pressure;
                let color = gases.color_for_gas(gas);

                // Brightness scales with pressure so pockets/voids are obvious.
                let intensity = (pressure.clamp(0.0, 200.0) / 200.0).clamp(0.1, 1.0);
                let r = (color.r as f32 * intensity) as u8;
                let g = (color.g as f32 * intensity) as u8;
                let b = (color.b as f32 * intensity) as u8;

                let screen_x = x as f32 * RENDER_BLOCK_WIDTH - camera.get_top_left(graphics).0 * RENDER_BLOCK_WIDTH;
                let screen_y = y as f32 * RENDER_BLOCK_WIDTH - camera.get_top_left(graphics).1 * RENDER_BLOCK_WIDTH;

                let rect = gfx::Rect::new(gfx::FloatPos(screen_x.round(), screen_y.round()), gfx::FloatSize(RENDER_BLOCK_WIDTH, RENDER_BLOCK_WIDTH));
                let colors = [gfx::Color::new(r, g, b, 140); 4];
                let tex_rect = gfx::Rect::new(gfx::FloatPos(0.0, 0.0), gfx::FloatSize(1.0, 1.0));
                rect_array.add_rect(&rect, &colors, &tex_rect);

                drawn += 1;
                if drawn >= max_cells {
                    break 'outer;
                }
            }
        }

        // A single solid white pixel texture lets us render colored, textured
        // rects without needing per-gas textures.
        static WHITE_PIXEL: std::sync::OnceLock<gfx::Texture> = std::sync::OnceLock::new();
        let white = WHITE_PIXEL.get_or_init(|| {
            let surface = gfx::Surface::new(gfx::IntSize(1, 1));
            gfx::Texture::load_from_surface(&surface)
        });

        rect_array.update();
        rect_array.render(graphics, Some(white), gfx::FloatPos(0.0, 0.0));

        Ok(())
    }
}
