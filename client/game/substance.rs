use anyhow::Result;

use crate::client::game::camera::Camera;
use crate::client::game::gases::ClientGases;
use crate::libraries::graphics as gfx;
use crate::shared::blocks::RENDER_BLOCK_WIDTH;

/// The maximum number of substance cells considered per frame, matching the
/// overlay framework's budget so a huge zoomed-out view can't blow up cost.
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
/// This is the high-fidelity *field-texture* renderer. Rather than drawing one
/// flat quad per tile (which made every block read as a single pixel), it
/// bakes the visible substance region into a small RGBA **field texture**
/// (1 texel = 1 tile) and draws ONE quad through the field shader. The GPU's
/// bilinear filter blurs adjacent tiles into a smooth, continuous body of fluid,
/// feathers boundary cells into soft edges, and the shader's sub-tile ripple
/// undulates the surface — so liquids read as wavy, cartoon water instead of
/// blocky cells.
///
/// The data source is the same single `GasLayer` that gases and liquids share
/// (liquids are just dense gas types), so there is no separate liquid path.
pub struct SubstanceRenderer {
    /// Rolling seconds for the shader's animation.
    time: gfx::SubstanceTime,
    /// GPU field texture (1 texel = 1 visible tile, RGBA). Re-uploaded each
    /// frame for the visible region. Set to 0 until first render (GL context
    /// may not be current at construction time).
    field_texture: u32,
    /// Region-quad buffers reused across frames.
    gas_quad: gfx::VertexBuffer,
    liquid_quad: gfx::VertexBuffer,
}

/// Opaque fill: the shader's multi-tap blur creates soft feathered edges and a
/// smooth surface from the hard opaque/empty transition, so cells no longer
/// render with a visible block-sized alpha outline band.
const FILL_ALPHA: u8 = 255;

impl SubstanceRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            time: gfx::SubstanceTime::new(),
            field_texture: 0,
            gas_quad: gfx::VertexBuffer::new(),
            liquid_quad: gfx::VertexBuffer::new(),
        }
    }

    /// Draws the visible substance region as a smooth, animated fluid field.
    pub fn render(&mut self, graphics: &mut gfx::GraphicsContext, camera: &Camera, gases: &ClientGases) -> Result<(), anyhow::Error> {
        let layer = gases.layer();
        let (world_w, world_h) = layer.get_size();
        if world_w == 0 || world_h == 0 {
            return Ok(());
        }

        let time = self.time.seconds();
        let (top_left_x, top_left_y) = camera.get_top_left(graphics);
        let (bottom_right_x, bottom_right_y) = camera.get_bottom_right(graphics);

        // Visible tile range, clamped to world bounds (same culling as v1).
        let start_x = i32::max(0, top_left_x.floor() as i32);
        let start_y = i32::max(0, top_left_y.floor() as i32);
        let end_x = i32::min(world_w as i32, bottom_right_x.ceil() as i32 + 1);
        let end_y = i32::min(world_h as i32, bottom_right_y.ceil() as i32 + 1);

        if end_x <= start_x || end_y <= start_y {
            return Ok(());
        }
        let region_w = end_x - start_x;
        let region_h = end_y - start_y;

        self.ensure_field_texture();

        // Per-gas appearance cache (distinct gas ids are few — ~6 per world).
        let mut appearance: std::collections::HashMap<i32, GasAppearance> = std::collections::HashMap::new();

        // Build the two field buffers (gases & liquids) as RGBA bytes, one texel
        // per visible tile, row-major from the top-left of the region.
        let mut gas_pixels: Vec<u8> = vec![0; (region_w * region_h * 4) as usize];
        let mut liquid_pixels: Vec<u8> = vec![0; (region_w * region_h * 4) as usize];

        // Hot path: iterate the dense cell array directly (flat index instead of
        // per-cell translate_coords + Result). Coords are already clamped to
        // world bounds above, and cells() is row-major over (x * height + y), so
        // this index math is exact. Precompute the base pointer once for speed.
        let cells = layer.cells();
        let map_h = world_h as i32; // map height (size.1) — dense row stride
        let mut scanned = 0usize;
        'outer: for y in start_y..end_y {
            let mut src = (start_x * map_h + y) as isize;
            // Output buffer offset for this row ((y - start_y) * region_w * 4).
            let mut out = ((y - start_y) * region_w * 4) as usize;
            for _x in start_x..end_x {
                // cannot panic: coords clamped to world bounds above
                #[allow(clippy::indexing_slicing)]
                {
                    let cell = cells[src as usize];
                    let id = cell.gas.raw();
                    let app = appearance.entry(id).or_insert_with(|| gases.appearance(cell.gas));

                    if app.renderable {
                        let c = app.color.set_a(FILL_ALPHA);
                        if app.liquid {
                            liquid_pixels[out] = c.r;
                            liquid_pixels[out + 1] = c.g;
                            liquid_pixels[out + 2] = c.b;
                            liquid_pixels[out + 3] = c.a;
                        } else {
                            gas_pixels[out] = c.r;
                            gas_pixels[out + 1] = c.g;
                            gas_pixels[out + 2] = c.b;
                            gas_pixels[out + 3] = c.a;
                        }
                    }

                    scanned += 1;
                    if scanned >= MAX_SUBSTANCE_CELLS {
                        break 'outer;
                    }
                }
                src += map_h as isize; // dense: x step jumps by map height
                out += 4;              // output texel stride (RGBA)
            }
        }

        let cell_size = gfx::FloatSize(RENDER_BLOCK_WIDTH, RENDER_BLOCK_WIDTH);
        let tile_origin = gfx::FloatSize(start_x as f32, start_y as f32);
        let field_size = gfx::FloatSize(region_w as f32, region_h as f32);

        // The region quad's screen-space corners. Tile (x,y) sits at
        // (x*BLOCK - top*BLOCK) on screen; its world (+screen-offset-free) pos is
        // (x*BLOCK, y*BLOCK) which we bake into the tex channel for the shader to
        // anchor noise and compute UVs. Built fresh each frame (camera moves).
        let screen_x0 = start_x as f32 * RENDER_BLOCK_WIDTH - top_left_x * RENDER_BLOCK_WIDTH;
        let screen_y0 = start_y as f32 * RENDER_BLOCK_WIDTH - top_left_y * RENDER_BLOCK_WIDTH;
        let screen_x1 = end_x as f32 * RENDER_BLOCK_WIDTH - top_left_x * RENDER_BLOCK_WIDTH;
        let screen_y1 = end_y as f32 * RENDER_BLOCK_WIDTH - top_left_y * RENDER_BLOCK_WIDTH;
        let world_x0 = start_x as f32 * RENDER_BLOCK_WIDTH;
        let world_y0 = start_y as f32 * RENDER_BLOCK_WIDTH;
        let world_x1 = end_x as f32 * RENDER_BLOCK_WIDTH;
        let world_y1 = end_y as f32 * RENDER_BLOCK_WIDTH;

        // Two draw calls: gases first (soft motes), then liquids (bubbles) on
        // top. Each uploads its own field buffer and draws the region quad.
        let has_gas = gas_pixels.iter().any(|&b| b != 0);
        let has_liquid = liquid_pixels.iter().any(|&b| b != 0);

        let screen_corners = [
            gfx::FloatPos(screen_x0, screen_y0),
            gfx::FloatPos(screen_x1, screen_y0),
            gfx::FloatPos(screen_x0, screen_y1),
            gfx::FloatPos(screen_x1, screen_y1),
        ];
        let world_corners = [
            gfx::FloatPos(world_x0, world_y0),
            gfx::FloatPos(world_x1, world_y0),
            gfx::FloatPos(world_x0, world_y1),
            gfx::FloatPos(world_x1, world_y1),
        ];

        if has_liquid {
            Self::upload_field(self.field_texture, &liquid_pixels, region_w, region_h);
            self.liquid_quad.build_region_quad(screen_corners, world_corners);
            self.liquid_quad.upload();
            graphics.render_field(&self.liquid_quad, self.field_texture, cell_size, tile_origin, field_size, time, true);
        }
        if has_gas {
            Self::upload_field(self.field_texture, &gas_pixels, region_w, region_h);
            self.gas_quad.build_region_quad(screen_corners, world_corners);
            self.gas_quad.upload();
            graphics.render_field(&self.gas_quad, self.field_texture, cell_size, tile_origin, field_size, time, false);
        }

        Ok(())
    }

    /// Uploads an RGBA field buffer (row-major, top-to-bottom) to the GPU field
    /// texture. The texture object keeps its LINEAR filter + wrapping from init,
    /// so a same-size re-upload just refreshes the data.
    fn upload_field(texture: u32, pixels: &[u8], w: i32, h: i32) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA as i32, w, h, 0, gl::RGBA, gl::UNSIGNED_BYTE, pixels.as_ptr().cast());
        }
    }


    /// Lazily creates the GPU field texture with LINEAR (bilinear) filtering and
    /// clamped wrapping, so field sampling feathers edges and stays in bounds.
    fn ensure_field_texture(&mut self) {
        if self.field_texture != 0 {
            return;
        }
        unsafe {
            gl::GenTextures(1, &mut self.field_texture);
            gl::BindTexture(gl::TEXTURE_2D, self.field_texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        }
    }
}

impl Default for SubstanceRenderer {
    fn default() -> Self {
        Self::new()
    }
}
