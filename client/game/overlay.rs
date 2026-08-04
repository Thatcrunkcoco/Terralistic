use crate::client::game::camera::Camera;
use crate::libraries::events::Event;
use crate::libraries::graphics as gfx;
use crate::libraries::graphics::BaseUiElement;
use crate::shared::blocks::RENDER_BLOCK_WIDTH;

/// A data source that a generic `Overlay` renders.
///
/// This is the seam that lets the whole overlay *framework* — the toggle, the
/// gray-wash, the culled/batched cell rendering, and the legend panel — be a
/// single reusable component, while each concrete overlay (gas, liquid,
/// electrical, plumbing, item transport, ...) only supplies two things:
///
///   * `cell_color(x, y)` — what tint (if any) to draw at a given tile. Returning
///     `None` means "draw nothing here" (e.g. empty space, or a cell that overlay
///     isn't interested in). Returning a `Color` draws that cell at that exact
///     color; the caller uses `color.a` as the alpha / brightness so a provider
///     can distinguish "faint background tint" from "vivid highlight" purely in
///     data rather than through renderer special-casing.
///   * `legend()` — the key panel: one colored swatch + label per notable entry.
///
/// The split mirrors the two data shapes we expect in the game:
///   * Flowing *substances* (gas / liquid) live in a shared layer and are exposed
///     through a provider that reads that layer.
///   * Static *networks* (electrical / plumbing / item transport) live in the
///     block / tile-entity space and are exposed through a provider that reads
///     per-tile block state.
///
/// Either way the `Overlay` component is identical, so adding a new view later is
/// just "write a provider" — no new overlay class, no duplicated toggle/wash/
/// legend/rendering code.
pub trait OverlayProvider {
    /// Human-readable overlay name (used for debug labels, HUD icon, etc.).
    fn name(&self) -> &str;

    /// Returns the color to draw at tile (x, y), or `None` to skip the cell.
    /// The returned color's alpha acts as the draw intensity / brightness.
    fn cell_color(&self, x: i32, y: i32) -> Option<gfx::Color>;

    /// The legend entries (colored swatch + label) shown in the overlay's panel.
    fn legend(&self) -> Vec<(String, gfx::Color)>;

    /// The size of the underlying data (width, height) in tiles, or `(0, 0)` if
    /// unknown. Used to clamp the visible-cell iteration so a zoomed-out view
    /// never scans past the data's real extent.
    fn world_size(&self) -> (u32, u32) {
        (0, 0)
    }
}

/// The maximum number of cells an overlay draws per frame, so a large zoomed-out
/// view can never blow up the per-frame render cost.
const MAX_CELLS_PER_FRAME: usize = 120_000;

/// A generic, reusable overlay view.
///
/// Owns the per-overlay *view state* that every overlay needs in common:
///   * an on/off state toggled with a hotkey and (in debug mode) an on-screen icon,
///   * a translucent gray wash over the terrain (the ONI-style desaturation),
///   * culled, batched rendering of every provider-highlighted cell in one draw call,
///   * a legend panel mapping colors to names.
///
/// It deliberately stores no data source and is provider-agnostic: the caller
/// passes a `&dyn OverlayProvider` into each render/event call. This keeps the
/// component decoupled from where the visualized data lives (a gas layer, a
/// liquid layer, a block/tile-entity electrical network, ...) and, because the
/// data source is borrowed only for the duration of a call, it composes cleanly
/// with game state that is constructed and mutated elsewhere (like the client's
/// mirrored gas layer).
pub struct Overlay {
    /// Whether the overlay is currently drawn on screen.
    open: bool,
    /// Whether we're in debug mode (shows the HUD toggle icon).
    debug: bool,
    /// The on-screen toggle button (HUD icon).
    toggle_button: gfx::Button,
    /// Tracks whether the mouse went down on the icon, for a genuine click.
    mouse_down_on_button: bool,
    /// Whether the HUD label reflects the current `open` state (avoid rebuilding
    /// the label texture every frame).
    label_state: bool,
}

impl Overlay {
    /// Creates an overlay with a G-hotkey toggle and (in debug mode) an on-screen
    /// toggle icon.
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
            mouse_down_on_button: false,
            label_state: false,
        }
    }

    /// Initializes the toggle icon's HUD label, using the provider's name.
    pub fn init(&mut self, graphics: &gfx::GraphicsContext, provider: &dyn OverlayProvider) {
        self.toggle_button.texture =
            gfx::Texture::load_from_surface(&graphics.font.create_text_surface(&format!("{}: OFF", provider.name()), None));
    }

    /// Whether the overlay is currently open.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Handles input for the overlay: the G hotkey (which toggles *this* overlay)
    /// and the on-screen toggle icon.
    ///
    /// Returns `Ok(true)` when this event was *consumed* by the UI: i.e. a mouse
    /// up/down landed on the toggle icon. The caller should then skip passing the
    /// event to world handlers.
    pub fn on_event(&mut self, event: &Event, graphics: &mut gfx::GraphicsContext) -> Result<bool, anyhow::Error> {
        if let Some(gfx_event) = event.downcast::<gfx::Event>() {
            if let gfx::Event::KeyPress(gfx::Key::G, false) = gfx_event {
                self.toggle();
                return Ok(false);
            }
            if self.debug && self.button_click(graphics, gfx_event)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Handles press/release of `MouseLeft`, returning `Ok(true)` when the event
    /// landed on the toggle icon (so the caller knows the UI consumed it).
    fn button_click(&mut self, graphics: &gfx::GraphicsContext, gfx_event: &gfx::Event) -> Result<bool, anyhow::Error> {
        match gfx_event {
            gfx::Event::KeyPress(gfx::Key::MouseLeft, _) => {
                let hovered = self.toggle_button.is_hovered(graphics, &gfx::Container::default(graphics));
                if hovered {
                    self.mouse_down_on_button = true;
                }
                Ok(hovered)
            }
            gfx::Event::KeyRelease(gfx::Key::MouseLeft, _) => {
                let was_down = self.mouse_down_on_button;
                self.mouse_down_on_button = false;
                let hovered = self.toggle_button.is_hovered(graphics, &gfx::Container::default(graphics));
                if was_down && hovered {
                    self.toggle();
                }
                Ok(was_down)
            }
            _ => Ok(false),
        }
    }

    /// Toggles the overlay on/off. Callers that need to react to the visibility
    /// change (e.g. start/stop live data updates) should check [`Overlay::is_open`]
    /// after events are processed.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Renders the overlay: gray-wash the terrain beneath, then draw the
    /// provider's highlighted cells, then the legend. Called right after the
    /// terrain is drawn but before world entities / HUD.
    pub fn render(
        &mut self,
        graphics: &mut gfx::GraphicsContext,
        camera: &Camera,
        provider: &dyn OverlayProvider,
    ) -> Result<(), anyhow::Error> {
        if self.open {
            self.render_gray_wash(graphics);
            self.render_cells(graphics, camera, provider)?;
            self.render_legend(graphics, provider)?;
        }
        Ok(())
    }

    /// Renders the on-screen toggle icon (HUD). Drawn after the world and HUD so it
    /// sits above everything.
    pub fn render_hud(&mut self, graphics: &mut gfx::GraphicsContext, provider: &dyn OverlayProvider) {
        if self.debug {
            if self.open != self.label_state {
                let label = if self.open { "ON" } else { "OFF" };
                self.toggle_button.texture =
                    gfx::Texture::load_from_surface(&graphics.font.create_text_surface(&format!("{}: {label}", provider.name()), None));
                self.label_state = self.open;
            }
            self.toggle_button.render(graphics, &gfx::Container::default(graphics));
        }
    }

    /// Paints a translucent neutral-gray rectangle over the whole viewport,
    /// desaturating the terrain toward monochrome so highlighted cells pop.
    fn render_gray_wash(&self, graphics: &gfx::GraphicsContext) {
        let viewport = gfx::Rect::new(gfx::FloatPos(0.0, 0.0), graphics.get_window_size());
        viewport.render(graphics, gfx::Color::new(90, 90, 90, 150));
    }

    /// Draws every provider-highlighted cell within the viewport as a solid
    /// rectangle, batched into a single `RectArray` draw call. Cells outside the
    /// viewport are skipped (via `iter_visible_cells`) so we never iterate the
    /// whole world, and the count is capped so a huge zoomed-out view can't blow
    /// up the per-frame cost.
    fn render_cells(&self, graphics: &gfx::GraphicsContext, camera: &Camera, provider: &dyn OverlayProvider) -> Result<(), anyhow::Error> {
        // A single solid white pixel texture lets us render colored, textured
        // rects without per-cell textures. It must be an opaque white pixel first:
        // `Surface::new` clears to transparent black, and the shader multiplies the
        // sampled texel by the vertex color, so a blank surface would make every
        // cell invisible.
        static WHITE_PIXEL: std::sync::OnceLock<gfx::Texture> = std::sync::OnceLock::new();
        let white = WHITE_PIXEL.get_or_init(|| {
            let mut surface = gfx::Surface::new(gfx::IntSize(1, 1));
            if let Ok(pixel) = surface.get_pixel_mut(gfx::IntPos(0, 0)) {
                *pixel = gfx::Color::new(255, 255, 255, 255);
            }
            gfx::Texture::load_from_surface(&surface)
        });

        // Compute the camera origin once (it's the same for every cell) rather
        // than inside the per-cell closure.
        let (topleft_x, topleft_y) = camera.get_top_left(graphics);

        let mut rect_array = gfx::RectArray::new();
        let mut drawn = 0usize;
        self.iter_visible_cells(graphics, camera, provider, |x, y| {
            if let Some(color) = provider.cell_color(x, y) {
                let screen_x = x as f32 * RENDER_BLOCK_WIDTH - topleft_x * RENDER_BLOCK_WIDTH;
                let screen_y = y as f32 * RENDER_BLOCK_WIDTH - topleft_y * RENDER_BLOCK_WIDTH;
                let rect = gfx::Rect::new(
                    gfx::FloatPos(screen_x.round(), screen_y.round()),
                    gfx::FloatSize(RENDER_BLOCK_WIDTH, RENDER_BLOCK_WIDTH),
                );
                let tex_rect = gfx::Rect::new(gfx::FloatPos(0.0, 0.0), gfx::FloatSize(1.0, 1.0));
                let colors = [color; 4];
                rect_array.add_rect(&rect, &colors, &tex_rect);
                drawn += 1;
            }
        });

        // Only issue a draw if we actually added at least one colored cell; an
        // empty `RectArray` would render nothing anyway, but this skips the
        // pointless upload when the viewport holds no highlighted cells.
        if drawn > 0 {
            rect_array.update();
            rect_array.render(graphics, Some(white), gfx::FloatPos(0.0, 0.0));
        }

        Ok(())
    }

    /// Iterates every tile within the camera's viewport (clamped to world bounds)
    /// up to `MAX_CELLS_PER_FRAME`, calling `f` for each visited tile. Returns the
    /// number visited. This is the shared culling/capping that guarantees no
    /// overlay ever scans the whole world or exceeds the per-frame budget.
    pub fn iter_visible_cells(
        &self,
        graphics: &gfx::GraphicsContext,
        camera: &Camera,
        provider: &dyn OverlayProvider,
        mut f: impl FnMut(i32, i32),
    ) -> usize {
        let (top_left_x, top_left_y) = camera.get_top_left(graphics);
        let (bottom_right_x, bottom_right_y) = camera.get_bottom_right(graphics);

        // Clamp to the provider's real extent (if known) and to world origin.
        let (world_w, world_h) = provider.world_size();
        let (w, h) = (world_w as i32, world_h as i32);
        let start_x = i32::max(0, top_left_x as i32);
        let start_y = i32::max(0, top_left_y as i32);
        let end_x = if w > 0 { i32::min(w, bottom_right_x as i32 + 1) } else { bottom_right_x as i32 + 1 };
        let end_y = if h > 0 { i32::min(h, bottom_right_y as i32 + 1) } else { bottom_right_y as i32 + 1 };

        let mut drawn = 0usize;
        'outer: for x in start_x..end_x {
            for y in start_y..end_y {
                f(x, y);
                drawn += 1;
                if drawn >= MAX_CELLS_PER_FRAME {
                    break 'outer;
                }
            }
        }
        drawn
    }

    /// Renders a small legend panel in the top-left corner: one row per legend
    /// entry, a colored swatch beside its label.
    fn render_legend(&self, graphics: &mut gfx::GraphicsContext, provider: &dyn OverlayProvider) -> Result<(), anyhow::Error> {
        let entries = provider.legend();
        if entries.is_empty() {
            return Ok(());
        }

        const SWATCH: f32 = 20.0;
        const ROW_GAP: f32 = 10.0;
        const PAD: f32 = 12.0;
        const TEXT_SCALE: f32 = 2.0;
        let margin = gfx::SPACING as f32;

        // Measure the panel so it fits the longest label, then draw the background.
        let mut rows_height = 0.0f32;
        let mut max_name_width = 0.0f32;
        for (name, _) in &entries {
            let text_size = graphics.font.get_text_size_scaled(name, TEXT_SCALE, None);
            max_name_width = max_name_width.max(text_size.0);
            rows_height += (SWATCH.max(text_size.1)) + ROW_GAP;
        }
        let panel_size = gfx::FloatSize(max_name_width + SWATCH + ROW_GAP + PAD * 2.0, rows_height + PAD * 2.0);
        let panel = gfx::Rect::new(gfx::FloatPos(margin, margin), panel_size);
        panel.render(graphics, gfx::Color::new(15, 15, 15, 190));

        let mut y = margin + PAD;
        for (name, color) in &entries {
            let swatch_rect = gfx::Rect::new(gfx::FloatPos(margin + PAD, y), gfx::FloatSize(SWATCH, SWATCH));
            swatch_rect.render(graphics, *color);
            graphics
                .font
                .render_text(graphics, name, gfx::FloatPos(margin + PAD + SWATCH + ROW_GAP, y - 2.0), TEXT_SCALE);
            let text_size = graphics.font.get_text_size_scaled(name, TEXT_SCALE, None);
            y += (SWATCH.max(text_size.1)) + ROW_GAP;
        }

        Ok(())
    }
}
