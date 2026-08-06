use anyhow::Result;

use crate::libraries::graphics as gfx;
use crate::libraries::graphics::shaders::compile_shader;
use crate::libraries::graphics::vertex_buffer::{DrawMode, Vertex, VertexBuffer};

/// Full-screen cel-outline post-process shader, applied to the composited scene
/// texture in `update_window()`.
///
/// This is the Paper-Mario / Cult-of-the-Lamb treatment for the *whole* frame:
/// the world, the player, items and fluids are all already drawn in order into
/// the offscreen `window_texture`. We then sample that scene and — in a single
/// fragment pass over a fullscreen quad —
///
/// 1. **Sobel edge detection** on luminance → where adjacent pixels differ in
///    brightness (a shape boundary), paint a chunky dark outline. This is what
///    makes characters and terrain read as flat cel borders instead of raw
///    edges.
/// 2. **Cel shading / saturation** boost so flat regions read as bright cartoon
///    color rather than washed-out dither.
/// 3. A very subtle full-frame **softening** so the pixel artwork stops looking
///    like hard NEAREST squares and eases toward the smooth ONI/CoTL contour
///    feel, without fully blurring away the block identity.
///
/// The shader keeps the scene's alpha: where the scene is transparent the output
/// is transparent, and the outline is only drawn over *opaque* scene pixels so
/// we never ring the empty background.
///
/// Cairo-style "chunky" border thickness is a function of the edge threshold
/// plus slight alpha expansion; tune `OUTLINE_WEIGHT` and `EDGE_MIN` below.
const VERTEX_SHADER_CODE: &str = "
#version 330 core

layout (location = 0) in vec2 vertex_position;      // NDC [-1, 1]
layout (location = 1) in vec4 vertex_color;         // unused (white)
layout (location = 2) in vec2 vertex_texture_coordinate; // [0, 1]

out vec2 tex_coord;

void main() {
    gl_Position = vec4(vertex_position, 0.0, 1.0);
    tex_coord = vertex_texture_coordinate;
}
";

const FRAGMENT_SHADER_CODE: &str = "
#version 330 core

in vec2 tex_coord;
out vec4 color;

uniform sampler2D scene_tex;
uniform vec2  texel_size;   // 1.0 / scene pixel dimensions
uniform float outline_weight; // 0..1 -> how strongly to darken detected edges
uniform float edge_min;     // minimum Sobel magnitude that counts as an edge
uniform float cel_strength; // saturation boost applied to flat regions
uniform float soften;       // 0..1 how much to blend toward a 3x3 blur

float lum(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

// 3x3 gaussian blur of the scene (used for the subtle soften).
vec4 soft(vec2 uv) {
    vec2 t = texel_size;
    float w[3] = float[](0.25, 0.5, 0.25);
    vec4 acc = vec4(0.0);
    float total = 0.0;
    for (int i = -1; i <= 1; i++) {
        for (int j = -1; j <= 1; j++) {
            float ww = w[abs(i)] * w[abs(j)];
            acc += texture(scene_tex, uv + vec2(float(i) * t.x, float(j) * t.y)) * ww;
            total += ww;
        }
    }
    return acc / total;
}

void main() {
    vec4 base = texture(scene_tex, tex_coord);

    // Transparent background stays transparent (let HUD / window show through).
    if (base.a < 0.01) {
        color = base;
        return;
    }

    vec2 t = texel_size;

    // 8 neighbor luminance samples (Sobel on a 3x3 grid).
    float l  = lum(texture(scene_tex, tex_coord + vec2(-t.x,  0.0)).rgb);
    float r  = lum(texture(scene_tex, tex_coord + vec2( t.x,  0.0)).rgb);
    float u  = lum(texture(scene_tex, tex_coord + vec2( 0.0, -t.y)).rgb);
    float d  = lum(texture(scene_tex, tex_coord + vec2( 0.0,  t.y)).rgb);
    float tl = lum(texture(scene_tex, tex_coord + vec2(-t.x, -t.y)).rgb);
    float tr = lum(texture(scene_tex, tex_coord + vec2( t.x, -t.y)).rgb);
    float bl = lum(texture(scene_tex, tex_coord + vec2(-t.x,  t.y)).rgb);
    float br = lum(texture(scene_tex, tex_coord + vec2( t.x,  t.y)).rgb);

    // Sobel gradient magnitude.
    float gx = (tr + 2.0 * r + br) - (tl + 2.0 * l + bl);
    float gy = (bl + 2.0 * d + br) - (tl + 2.0 * u + tr);
    float mag = length(vec2(gx, gy));

    // Edge -> outline: a soft band around edges so it looks like a drawn border
    // rather than anti-aliased line.
    float edge = smoothstep(edge_min, edge_min + 0.12, mag);

    // Combine: keep the scene color, but mix toward a dark ink color where an
    // edge is detected, and slightly widen the border with a faint outer halo.
    vec3 ink = vec3(0.05, 0.04, 0.05);
    vec3 outlined = mix(base.rgb, ink, edge * outline_weight);

    // CEL boost: push saturation + a touch of brightness on flat (non-edge)
    // regions so they read as bright, flat cartoon color.
    float lout = lum(outlined);
    vec3 cel = mix(vec3(lout), outlined, 1.0 + cel_strength);
    cel = clamp(cel, 0.0, 1.0);

    // Subtle softening blended in only on flat regions (never across edges) so
    // the pixel steps ease without smearing the outlines.
    vec3 softened = mix(cel, soft(tex_coord).rgb, soften * (1.0 - edge));

    color = vec4(softened, base.a);
}
";

/// Context for the cartoon cel-outline post-process: the compiled program, its
/// uniform locations, and a reused fullscreen quad (two triangles covering NDC
/// [-1,1]). The scene texture (`window_texture`) is bound as `scene_tex` and the
/// quad drawn to the default framebuffer (the window) in `update_window()`.
pub struct CartoonOutlineShader {
    pub program: u32,
    pub scene_tex: i32,
    pub texel_size: i32,
    pub outline_weight: i32,
    pub edge_min: i32,
    pub cel_strength: i32,
    pub soften: i32,
    pub fullscreen_quad: VertexBuffer,
}

impl CartoonOutlineShader {
    /// Compiles the cartoon shader and fetches uniform locations.
    pub(crate) fn new() -> Result<Self> {
        let program = compile_shader(VERTEX_SHADER_CODE, FRAGMENT_SHADER_CODE)?;

        let scene_tex = unsafe { gl::GetUniformLocation(program, c"scene_tex".as_ptr().cast::<i8>()) };
        let texel_size = unsafe { gl::GetUniformLocation(program, c"texel_size".as_ptr().cast::<i8>()) };
        let outline_weight = unsafe { gl::GetUniformLocation(program, c"outline_weight".as_ptr().cast::<i8>()) };
        let edge_min = unsafe { gl::GetUniformLocation(program, c"edge_min".as_ptr().cast::<i8>()) };
        let cel_strength = unsafe { gl::GetUniformLocation(program, c"cel_strength".as_ptr().cast::<i8>()) };
        let soften = unsafe { gl::GetUniformLocation(program, c"soften".as_ptr().cast::<i8>()) };

        // Fullscreen quad: NDC corners with matching [0,1] tex coords.
        let mut quad = VertexBuffer::new();
        let white = gfx::Color::new(255, 255, 255, 255);
        let corners = [
            (gfx::FloatPos(-1.0, -1.0), gfx::FloatPos(0.0, 0.0)), // bottom-left
            (gfx::FloatPos(1.0, -1.0), gfx::FloatPos(1.0, 0.0)), // bottom-right
            (gfx::FloatPos(-1.0, 1.0), gfx::FloatPos(0.0, 1.0)), // top-left
            (gfx::FloatPos(1.0, 1.0), gfx::FloatPos(1.0, 1.0)), // top-right
        ];
        for &i in &[0usize, 1, 2, 1, 3, 2] {
            quad.add_vertex(&Vertex { pos: corners[i].0, color: white, tex_pos: corners[i].1 });
        }
        quad.upload();

        Ok(Self { program, scene_tex, texel_size, outline_weight, edge_min, cel_strength, soften, fullscreen_quad: quad })
    }

    /// Draws the scene texture through this shader to the current framebuffer,
    /// covering the full viewport. Re-binds the passthrough shader afterward to
    /// preserve the renderer invariant.
    pub(crate) fn render(
        &self,
        graphics: &gfx::GraphicsContext,
        scene_texture: u32,
        viewport_w: u32,
        viewport_h: u32,
    ) {
        unsafe {
            gl::UseProgram(self.program);
            gl::Uniform1i(self.scene_tex, 0);
            gl::Uniform2f(self.texel_size, 1.0 / viewport_w as f32, 1.0 / viewport_h as f32);
            gl::Uniform1f(self.outline_weight, 0.85);
            gl::Uniform1f(self.edge_min, 0.32);
            gl::Uniform1f(self.cel_strength, 0.25);
            gl::Uniform1f(self.soften, 0.35);

            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, scene_texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        }

        // Draw the fullscreen quad with texture (enables attribute 2 = tex coords).
        self.fullscreen_quad.draw(true, DrawMode::Triangles);

        unsafe {
            gl::UseProgram(graphics.passthrough_shader.passthrough_shader);
        }
    }
}
