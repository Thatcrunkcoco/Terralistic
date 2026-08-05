use anyhow::Result;

use crate::libraries::graphics as gfx;
use crate::libraries::graphics::shaders::compile_shader;
use crate::libraries::graphics::vertex_buffer::{Vertex, VertexBuffer};
use crate::libraries::graphics::Color;

/// GLSL for the procedural "cartoon substance" renderer.
///
/// Two things make liquids/gases read as *cartoony* rather than flat-filled:
///
/// 1. **Animated roil** — the fragment's sampling position is wobbled by a pair
///    of value-noise hash functions that take world position + a rolling `time`
///    uniform. Gases billow and liquid surfaces gurgle without any keyframes.
///
/// 2. **Bubbles / motes** — a second hash layer scatters bright drifting specks
///    for liquids (and faint motes for gases), so a body of water has gentle
///    rising bubbles instead of being a static slab of color.
///
/// The per-substance base color is supplied per-vertex (CPU picks a ramp per gas
/// id), then the fragment modulates brightness/alpha with the noise so each cell
/// keeps its identity while the whole field shimmers and shifts.
///
/// All vector math is done in *world pixel* space so the wobble is resolution
/// independent and stays glued to the world as the camera moves/zooms.
const VERTEX_SHADER_CODE: &str = "
#version 330 core

layout (location = 0) in vec2 vertex_position;
layout (location = 1) in vec4 vertex_color;
layout (location = 2) in vec2 vertex_texture_coordinate;

out vec4 fragment_color;
out vec2 world_pos;

uniform mat3 transform_matrix;

void main() {
    gl_Position = vec4(transform_matrix * vec3(vertex_position, 1.f), 1.f);
    fragment_color = vertex_color;
    // The caller bakes each cell's *world-space* footprint into attribute 2
    // (tex coords), so we forward it straight to the fragment shader. Anchoring
    // the noise to world space keeps the wobble glued to the tiles as the
    // camera moves and zooms, rather than shimmering relative to the viewport.
    world_pos = vertex_texture_coordinate;
}
";

const FRAGMENT_SHADER_CODE: &str = "
#version 330 core

in vec4 fragment_color;
in vec2 world_pos;

layout(location = 0) out vec4 color;

uniform float time;        // seconds, rolling — drives the roil/mote animation
uniform vec2  cell_size;   // world-space size of one substance cell
uniform int   has_bubbles; // 1 => draw rising bubbles/motes for this substance

// --- small integer hashing so we can derive pseudo-random values deterministically
float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

// A smooth value-noise with a few octaves folded together.
float wobble(vec2 p, float t) {
    vec2 q = p;
    float w = 0.0;
    w += sin(q.x * 0.8 + t * 0.7) * sin(q.y * 0.9 - t * 0.5);
    w += 0.5 * sin(q.x * 1.7 - t * 1.2 + q.y * 1.3);
    w += 0.25 * sin(q.x * 3.1 + t * 1.9 - q.y * 2.2);
    return w / 1.75; // ~[-1..1]
}

float hash_noise(vec2 p) {
    return hash21(floor(p * 0.5));
}

void main() {
    // Which cell this fragment belongs to (world pixel space -> cell coords).
    vec2 cell = floor(world_pos / cell_size);

    float t = time;
    // Per-cell phase so neighbouring cells drift slightly out of sync (feels
    // like a gas roiling rather than a uniform pulse).
    float phase = hash21(cell);

    // 1) Animated roil: nudge brightness up/down smoothly per cell.
    float roil = wobble(cell * 0.35 + vec2(0.0, phase), t);

    // Drift the sampling grid a touch so edges shimmer instead of sitting still.
    float drift = sin(t * 0.6 + phase * 6.2831) * 0.08;

    // 2) Bubbles / motes for liquids. A layered hash over the cell gives a few
    //    bright spots; the time term makes them rise and twinkle. Gases get a
    //    faint, slower shimmer instead.
    float mote = 0.0;
    if (has_bubbles == 1) {
        float rise = fract(hash21(cell + floor(vec2(-t * 0.4, t * 0.5))) * 1.0);
        float m1 = smoothstep(0.0, 0.05, hash21(cell + vec2(t * 0.1)));
        mote = m1 * rise;
        mote = pow(mote, 3.0) * 1.6;
    } else {
        float m2 = hash_noise(cell + vec2(t * 0.05));
        mote = smoothstep(0.96, 1.0, m2) * 0.25;
    }

    // 3) Combine: base color brightened by roil, plus motes as additive white
    //    highlights so the surface gets sparkle while keeping the base hue.
    float bright = 0.82 + roil * 0.18 + drift;
    vec3 base = fragment_color.rgb * bright;
    vec3 final = base + vec3(mote, mote * 0.82, mote * 0.6);

    // Preserve the substance's opacity (set on CPU per substance) but let the
    // roil cheaply soften cells that sit at the field boundary. Fragments near
    // a cell edge fade a little so adjacent cells bleb into each other.
    float edge_fade = 0.85 + 0.15 * sin(phase * 3.14);
    float alpha = fragment_color.a * clamp(edge_fade + roil * 0.15, 0.0, 1.0);

    color = vec4(final, alpha);
}
";

/// Context holding the compiled substance shader and its uniform locations, plus
/// a unit-quad vertex buffer reused for both the tests of the shader pipeline and
/// any direct (single-sprite) draws. Batched substance cells are drawn through a
/// `VertexBuffer` of interleaved vertices using this program.
pub struct SubstanceShader {
    pub program: u32,
    pub time: i32,
    pub cell_size: i32,
    pub has_bubbles: i32,
    pub transform_matrix: i32,
    pub rect_vertex_buffer: VertexBuffer,
}

impl SubstanceShader {
    /// Compiles the substance program and fetches uniform locations.
    pub(crate) fn new() -> Result<Self> {
        let program = compile_shader(VERTEX_SHADER_CODE, FRAGMENT_SHADER_CODE)?;

        let time = unsafe { gl::GetUniformLocation(program, c"time".as_ptr().cast::<i8>()) };
        let cell_size = unsafe { gl::GetUniformLocation(program, c"cell_size".as_ptr().cast::<i8>()) };
        let has_bubbles = unsafe { gl::GetUniformLocation(program, c"has_bubbles".as_ptr().cast::<i8>()) };
        let transform_matrix = unsafe { gl::GetUniformLocation(program, c"transform_matrix".as_ptr().cast::<i8>()) };

        let mut rect_vertex_buffer = VertexBuffer::new();
        for pos in [
            gfx::FloatPos(0.0, 0.0),
            gfx::FloatPos(1.0, 0.0),
            gfx::FloatPos(0.0, 1.0),
            gfx::FloatPos(1.0, 0.0),
            gfx::FloatPos(1.0, 1.0),
            gfx::FloatPos(0.0, 1.0),
        ] {
            rect_vertex_buffer.add_vertex(&Vertex {
                pos,
                color: Color { r: 255, g: 255, b: 255, a: 255 },
                tex_pos: pos,
            });
        }
        rect_vertex_buffer.upload();

        Ok(Self { program, time, cell_size, has_bubbles, transform_matrix, rect_vertex_buffer })
    }
}
