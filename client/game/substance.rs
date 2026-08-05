use anyhow::Result;

use crate::client::game::camera::Camera;
use crate::client::game::gases::ClientGases;
use crate::libraries::graphics as gfx;
use crate::shared::blocks::RENDER_BLOCK_WIDTH;

/// The maximum number of substance cells drawn per frame, matching the overlay
/// framework's budget so a huge zoomed-out view can't blow up per-frame cost.
const MAX_SUBSTANCE_CELLS: usize = 120_000;

/// Per-gas rendering attributes resolved once per distinct gas id per frame.
#[derive(Clone, Copy)]
pub(crate) struct GasAppearance {
    /// Whether this gas should be drawn at all (skips vacuum and plain "air").
    pub(crate) renderable: bool,
    /// Whether it renders as a dense liquid (bubbles + solid body) vs a gas.
    pub(crate) liquid: bool,
    /// Its base color for the shader's per-cell fill.
    pub(crate) color: gfx::Color,
}

/// Renders the shared substance layer (gases *and* liquids) as an always-on,
/// cartoon-styled fluid display.
///
/// Unlike the flat debug overlay (solid per-cell rectangles with no motion),
/// this renderer feeds every visible cell through the procedural *substance*
/// shader: per-substance color, animated roil (gases billow, liquid surfaces
/// gurgle), rising bubbles for liquids, and soft alpha edges. The data source is
/// the same single `GasLayer` that gases and liquids share — liquids are just
/// dense gas types — so there is no separate liquid path to maintain.
///
/// The renderer batches all cells into two draw calls (gases, then liquids) so
/// the bubble treatment can differ per phase, and reuses the viewport culling
/// logic to only draw what's on screen.
pub struct SubstanceRenderer {
    /// Rolling seconds for the shader's animation; owned here so the wobble
    /// advances smoothly across frames.
    time: gfx::SubstanceTime,
}

impl SubstanceRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            time: gfx::SubstanceTime::new(),
        }
    }

    /// Draws all visible, renderable substance cells over the terrain.
    ///
    /// `gases` supplies both the layer data and the per-gas classification /
    /// color. Called right after terrain (background/walls/blocks) and before
    /// players/items so fluids sit beneath entities like a backdrop layer.
    pub fn render(&mut self, graphics: &mut gfx::GraphicsContext, camera: &Camera, gases: &ClientGases) -> Result<(), anyhow::Error> {
        let layer = gases.layer();
        let (world_w, world_h) = layer.get_size();
        if world_w == 0 || world_h == 0 {
            return Ok(());
        }

        let time = self.time.seconds();

        // One batch per phase: gases (soft motes), then liquids (bubbles). Each
        // rect carries its world-space footprint in the tex channel for the
        // fragment noise, and its per-cell base color in the vertex color.
        //
        // Per-gas appearance is computed once per distinct gas id and cached for
        // the frame (gas ids are dense, small indices), so we don't re-lock the
        // gas registry for every visible cell.
        let mut gas_batch = gfx::RectArray::new();
        let mut liquid_batch = gfx::RectArray::new();
        let mut gas_count = 0usize;
        let mut liquid_count = 0usize;
        let mut appearance: std::collections::HashMap<i32, GasAppearance> = std::collections::HashMap::new();

        self.iter_visible_cells(graphics, camera, world_w as i32, world_h as i32, |x, y| {
            let cell = match layer.get_cell(x, y) {
                Ok(c) => c,
                Err(_) => return,
            };
            let id = cell.gas.raw();
            let app = appearance.entry(id).or_insert_with(|| gases.appearance(cell.gas));
            if !app.renderable {
                return;
            }

            // Screen-space rect for on-screen placement (matches the overlay's
            // viewport-relative coordinate math).
            let (top_left_x, top_left_y) = camera.get_top_left(graphics);
            let screen_x = x as f32 * RENDER_BLOCK_WIDTH - top_left_x * RENDER_BLOCK_WIDTH;
            let screen_y = y as f32 * RENDER_BLOCK_WIDTH - top_left_y * RENDER_BLOCK_WIDTH;
            let rect = gfx::Rect::new(
                gfx::FloatPos(screen_x.round(), screen_y.round()),
                gfx::FloatSize(RENDER_BLOCK_WIDTH, RENDER_BLOCK_WIDTH),
            );

            // World-space footprint goes into the tex channel (the fragment shader
            // reads it as `world_pos`). Anchoring noise to world space keeps the
            // wobble glued to the world as the camera moves.
            let world_rect = gfx::Rect::new(
                gfx::FloatPos(x as f32 * RENDER_BLOCK_WIDTH, y as f32 * RENDER_BLOCK_WIDTH),
                gfx::FloatSize(RENDER_BLOCK_WIDTH, RENDER_BLOCK_WIDTH),
            );

            // Soft edges: interior cells (the same substance all around) render
            // more opaque; boundary cells (adjacent to empty space or a different
            // substance) fade out so gas pockets billow and liquid surfaces soften
            // instead of ending in a hard wall of color.
            let interior = Self::is_interior(layer, x, y, cell.gas);
            let alpha = if interior { 225 } else { 110 };
            let base_color = app.color.set_a(alpha);
            let colors = [base_color; 4];

            if app.liquid {
                liquid_batch.add_rect(&rect, &colors, &world_rect);
                liquid_count += 1;
            } else {
                gas_batch.add_rect(&rect, &colors, &world_rect);
                gas_count += 1;
            }
        });

        let cell_size = gfx::FloatSize(RENDER_BLOCK_WIDTH, RENDER_BLOCK_WIDTH);
        if gas_count > 0 {
            gas_batch.update();
            gas_batch.render_substance(graphics, cell_size, false, time);
        }
        if liquid_count > 0 {
            liquid_batch.update();
            liquid_batch.render_substance(graphics, cell_size, true, time);
        }

        Ok(())
    }

    /// Whether a cell is fully surrounded (cardinal neighbors) by the same
    /// substance. Interior cells render more opaque (a solid body of fluid);
    /// boundary cells fade to create a soft, billowing edge. `layer` may extend
    /// beyond the visible-viewport cells being drawn, and both bounds and
    /// neighbor content are checked.
    fn is_interior(layer: &crate::shared::gases::GasLayer, x: i32, y: i32, gas: crate::shared::gases::GasId) -> bool {
        for (nx, ny) in [(x, y - 1), (x, y + 1), (x - 1, y), (x + 1, y)] {
            match layer.get_cell(nx, ny) {
                Ok(neighbor) => {
                    if neighbor.gas != gas {
                        return false;
                    }
                }
                // Out of world bounds counts as a boundary: fade the edge cell.
                Err(_) => return false,
            }
        }
        true
    }

    /// Iterates every tile within the camera's viewport (clamped to world bounds)
    /// up to `MAX_SUBSTANCE_CELLS`, calling `f` for each visited tile. Mirrors the
    /// overlay framework's culling/capping so we never scan the whole world.
    fn iter_visible_cells(
        &self,
        graphics: &gfx::GraphicsContext,
        camera: &Camera,
        world_w: i32,
        world_h: i32,
        mut f: impl FnMut(i32, i32),
    ) {
        let (top_left_x, top_left_y) = camera.get_top_left(graphics);
        let (bottom_right_x, bottom_right_y) = camera.get_bottom_right(graphics);

        let start_x = i32::max(0, top_left_x as i32);
        let start_y = i32::max(0, top_left_y as i32);
        let end_x = i32::min(world_w, bottom_right_x as i32 + 1);
        let end_y = i32::min(world_h, bottom_right_y as i32 + 1);

        let mut drawn = 0usize;
        'outer: for x in start_x..end_x {
            for y in start_y..end_y {
                f(x, y);
                drawn += 1;
                if drawn >= MAX_SUBSTANCE_CELLS {
                    break 'outer;
                }
            }
        }
    }
}

impl Default for SubstanceRenderer {
    fn default() -> Self {
        Self::new()
    }
}
