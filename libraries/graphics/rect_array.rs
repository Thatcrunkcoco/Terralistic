use crate::libraries::graphics as gfx;

use super::vertex_buffer::{DrawMode, Vertex, VertexBuffer};

/// The struct `RectArray` is used to draw multiple rectangles with the same texture
/// and in one draw call. This is much faster than drawing each rectangle individually.
pub struct RectArray {
    vertex_buffer: VertexBuffer,
}

/// Lazily-initialised monotonic "time" reference used to drive the procedural
/// substance animation. Each fragment reads a *rolling* seconds value (not a raw
/// wall-clock, to avoid jumps) so gas roil and bubbles animate smoothly.
pub struct SubstanceTime {
    start: std::time::Instant,
    last: f32,
}

impl SubstanceTime {
    /// Creates a fresh clock starting at `0.0`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
            last: 0.0,
        }
    }

    /// Returns the current rolling time in seconds since construction.
    #[must_use]
    pub fn seconds(&mut self) -> f32 {
        self.last = self.start.elapsed().as_secs_f32();
        self.last
    }
}

impl RectArray {
    /// Creates a new `RectArray`.
    #[must_use]
    pub fn new() -> Self {
        Self { vertex_buffer: VertexBuffer::new() }
    }

    /// Adds a rectangle to the `RectArray`.
    pub fn add_rect(&mut self, rect: &gfx::Rect, colors: &[gfx::Color; 4], tex_rect: &gfx::Rect) {
        let top_left = rect.pos;
        let top_right = rect.pos + gfx::FloatSize(rect.size.0, 0.0);
        let bottom_left = rect.pos + gfx::FloatSize(0.0, rect.size.1);
        let bottom_right = rect.pos + rect.size;

        let tex_top_left = tex_rect.pos;
        let tex_top_right = tex_rect.pos + gfx::FloatSize(tex_rect.size.0, 0.0);
        let tex_bottom_left = tex_rect.pos + gfx::FloatSize(0.0, tex_rect.size.1);
        let tex_bottom_right = tex_rect.pos + tex_rect.size;

        // first triangle
        self.vertex_buffer.add_vertex(&Vertex {
            pos: top_left,
            color: colors[0],
            tex_pos: tex_top_left,
        });

        self.vertex_buffer.add_vertex(&Vertex {
            pos: top_right,
            color: colors[1],
            tex_pos: tex_top_right,
        });

        self.vertex_buffer.add_vertex(&Vertex {
            pos: bottom_left,
            color: colors[2],
            tex_pos: tex_bottom_left,
        });

        // second triangle
        self.vertex_buffer.add_vertex(&Vertex {
            pos: top_right,
            color: colors[1],
            tex_pos: tex_top_right,
        });

        self.vertex_buffer.add_vertex(&Vertex {
            pos: bottom_right,
            color: colors[3],
            tex_pos: tex_bottom_right,
        });

        self.vertex_buffer.add_vertex(&Vertex {
            pos: bottom_left,
            color: colors[2],
            tex_pos: tex_bottom_left,
        });
    }

    pub fn update(&self) {
        self.vertex_buffer.upload();
    }

    /// Draws the batched rects through the procedural *substance* shader instead
    /// of the passthrough shader.
    ///
    /// This is how liquids/gases get their cartoon look: the caller bakes each
    /// cell's world-space position into the `tex_pos` channel of every vertex
    /// (so the fragment shader can anchor noise to world space), plus a
    /// per-substance base color in the vertex color. The shader then applies
    /// animated roil, bubbles/motes, and soft alpha to turn flat cells into a
    /// shimmering body of fluid.
    ///
    /// `cell_size` is the world-space size of one cell (used to derive cell
    /// coords from world positions in the shader). `has_bubbles` toggles the
    /// rising-bubble layer (liquids) vs the faint gas-mote shimmer.
    ///
    /// The passthrough shader is restored afterward, matching the rest of the
    /// renderer's invariant that passthrough is the "current" program.
    pub fn render_substance(&self, graphics: &gfx::GraphicsContext, cell_size: gfx::FloatSize, has_bubbles: bool, time: f32) {
        unsafe {
            gl::UseProgram(graphics.substance_shader.program);
            gl::Uniform1f(graphics.substance_shader.time, time);
            gl::Uniform2f(graphics.substance_shader.cell_size, cell_size.0, cell_size.1);
            gl::Uniform1i(graphics.substance_shader.has_bubbles, i32::from(has_bubbles));
            gl::UniformMatrix3fv(graphics.substance_shader.transform_matrix, 1, gl::FALSE, graphics.normalization_transform.matrix.as_ptr());
        }
        // Draw with `has_texture = true` purely so attribute 2 (tex_pos, which
        // carries the world position for the fragment noise) is enabled.
        self.vertex_buffer.draw(true, DrawMode::Triangles);

        unsafe {
            gl::UseProgram(graphics.passthrough_shader.passthrough_shader);
        }
    }

    /// Draws the `RectArray`.
    pub fn render(&self, graphics: &gfx::GraphicsContext, texture: Option<&gfx::Texture>, pos: gfx::FloatPos) {
        // to avoid artifacts
        let pos = gfx::FloatPos(pos.0 + 0.01, pos.1 + 0.01);

        unsafe {
            let mut transform = graphics.normalization_transform.clone();

            transform.translate(pos);

            gl::UniformMatrix3fv(graphics.passthrough_shader.transform_matrix, 1, gl::FALSE, transform.matrix.as_ptr());

            texture.map_or_else(
                || {
                    gl::Uniform1i(graphics.passthrough_shader.has_texture, 0);
                },
                |texture| {
                    transform = texture.get_normalization_transform();
                    gl::UniformMatrix3fv(graphics.passthrough_shader.texture_transform_matrix, 1, gl::FALSE, transform.matrix.as_ptr());
                    gl::Uniform1i(graphics.passthrough_shader.has_texture, 1);
                    gl::BindTexture(gl::TEXTURE_2D, texture.texture_handle);
                },
            );

            gl::Uniform4f(graphics.passthrough_shader.global_color, 1.0, 1.0, 1.0, 1.0);

            self.vertex_buffer.draw(texture.is_some(), DrawMode::Triangles);
        }
    }
}
