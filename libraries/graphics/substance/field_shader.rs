use anyhow::Result;

use crate::libraries::graphics::shaders::compile_shader;
use crate::libraries::graphics::vertex_buffer::VertexBuffer;

/// GLSL for the *field-texture* substance renderer — the high-fidelity path of
/// the cartoon fluid system.
///
/// Where the per-tile renderer draws one quad per cell (so each 16px tile reads
/// as a single block), this shader draws ONE quad over the whole visible region
/// and samples a **field texture** (1 texel = 1 tile, precomputed RGBA color +
/// alpha on the CPU). The GPU's LINEAR filter performs *bilinear interpolation*
/// between texels, which gives two big wins for free:
///
/// 1. **Smooth interiors** — adjacent tiles of the same substance blend into a
///    continuous body of color instead of pixelated blocks.
/// 2. **Feathered edges** — where a boundary cell (low alpha) meets interior
///    cells (high alpha), the bilinear filter creates a soft, gradual fade.
///
/// On top of that the fragment shader adds:
///
/// - **Sub-tile animated surface waves**: the *fractional* sample position is
///   displaced by smooth sine octaves anchored to world tile coords, so the
///   liquid surface undulates continuously across texels rather than snapping
///   per tile.
/// - **Animated roil / bubbles / motes** carried over from the per-tile shader,
///   now modulated on the smooth field rather than flat per-tile colors.
///
/// All vector math stays in *world* space so the animation is resolution
/// independent and glued to the world as the camera moves/zooms.
const FIELD_VERTEX_SHADER: &str = "
#version 330 core

layout (location = 0) in vec2 vertex_position;        // screen-space world pixels
layout (location = 1) in vec4 vertex_color;           // unused (white)
layout (location = 2) in vec2 vertex_texture_coordinate; // world pixel pos of vertex

out vec2 v_uv;
out vec2 v_tile;

uniform mat3 transform_matrix;
uniform vec2 cell_size;
uniform vec2 tile_origin;   // minimum tile index covered by the field texture
uniform vec2 field_size;    // (cols, rows) tiles in the field texture

void main() {
    gl_Position = vec4(transform_matrix * vec3(vertex_position, 1.0), 1.0);

    vec2 world = vertex_texture_coordinate;      // world pixels
    vec2 tile_f = world / cell_size;             // fractional tile coords

    v_tile = tile_f;
    // Map fractional tile -> texture coords. The field is smoothed/billinear
    // upsampled so sampling at any fractional tile coordinate blends the
    // surrounding cells into a continuous, soft fluid body.
    v_uv = (tile_f - tile_origin + 0.5) / field_size;
}
";

const FIELD_FRAGMENT_SHADER: &str = "
#version 330 core

in vec2 v_uv;
in vec2 v_tile;

uniform sampler2D field_tex;
uniform float time;
uniform vec2  field_size;   // (cols, rows) tiles in the field texture
uniform int   has_bubbles;

layout(location = 0) out vec4 color;

float wobble(vec2 p, float t) {
    vec2 q = p;
    float w = 0.0;
    w += sin(q.x * 0.8 + t * 0.7) * sin(q.y * 0.9 - t * 0.5);
    w += 0.5 * sin(q.x * 1.7 - t * 1.2 + q.y * 1.3);
    w += 0.25 * sin(q.x * 3.1 + t * 1.9 - q.y * 2.2);
    return w / 1.75;
}

/// Multi-tap gaussian sample of the field. The field is 1 texel per tile, so a
/// single bilinear sample leaves hard block edges (the \"blocky\" look). Blurring
/// the field across neighboring texels turns the blocky staircase into soft,
/// smooth, gooey contours.
///
/// Used to be a 5x5 kernel (25 taps) but that's the single hottest part of the
/// fragment shader — every visible pixel sampled 25 neighboring texels, twice
/// (gas + liquid passes), which is what spun the GPU fans up. The bilinear
/// filter already does the coarse smoothing of tile steps, so a tight 3x3
/// gaussian (9 taps) still melts the blocky staircase into soft gooey contours
/// while cutting per-pixel texture traffic ~2.8x.
vec4 sampleField(vec2 uv) {
    vec2 step = 1.0 / max(field_size, vec2(1.0));
    float wg[3] = float[](0.25, 0.5, 0.25);
    vec4 acc = vec4(0.0);
    float total = 0.0;
    for (int i = -1; i <= 1; i++) {
        for (int j = -1; j <= 1; j++) {
            float w = wg[abs(i)] * wg[abs(j)];
            acc += texture(field_tex, uv + vec2(float(i) * step.x, float(j) * step.y)) * w;
            total += w;
        }
    }
    return acc / total;
}

void main() {
    // Wavy fluid surface + gentle body undulation: displace the sample position
    // by smooth world-anchored sine octaves (in tile units). The dominant Y term
    // drives a travelling wave on the liquid top; the thinner X term and the
    // second Y term add secondary ripple so it feels alive.
    vec2 disp;
    disp.x = 0.12 * sin(v_tile.y * 1.2 + time * 1.1)
           + 0.08 * sin(v_tile.y * 3.1 - time * 1.4);
    disp.y = 0.60 * sin(v_tile.x * 0.8 + time * 1.5)
           + 0.38 * sin(v_tile.x * 2.1 - time * 2.0 + v_tile.y * 0.4)
           + 0.20 * sin(v_tile.x * 4.5 + time * 2.6);

    vec2 sample_uv = v_uv + disp / max(field_size, vec2(1.0));
    vec4 field = sampleField(clamp(sample_uv, 0.0, 1.0));

    float roil = wobble(v_tile * 0.35, time);

    // Gases stay faint and milky; liquids run the *same* smooth mechanics but get
    // a stronger base so water/magma pop as vivid, saturated bodies.
    float bright = (has_bubbles == 1) ? (1.06 + roil * 0.18) : (0.85 + roil * 0.15);

    // Continuous, non-quantized motes so neither gas nor liquid ever snaps to a
    // per-tile grid (the old integer-cell hash was what made liquid chunk).
    float mote = 0.0;
    if (has_bubbles == 1) {
        // Thicker liquid: soft lobed bubbles drifting upward on smooth coords.
        vec2 bp = v_tile * 2.1 - vec2(0.0, time * 0.7);
        float b1 = 0.5 + 0.5 * sin(wobble(bp, time * 1.2) * 6.2831 + time * 0.8);
        float b2 = 0.5 + 0.5 * sin(wobble(bp + vec2(3.1, 1.7), time * 0.9) * 6.2831 - time * 0.6);
        mote = pow(max(b1, b2), 3.0) * 1.05;
    } else {
        // Faint gas sparkle — non-quantized continuous noise (the fluid-style
        // fix) so no per-tile grid ever snaps. Two fine lobes of soft banding,
        // kept low amplitude so gas stays faint and milky.
        vec2 gp = v_tile * 0.9 + vec2(time * 0.04, -time * 0.02);
        float n1 = 0.5 + 0.5 * sin(wobble(gp, time * 0.5) * 6.2831 + time * 0.6);
        float n2 = 0.5 + 0.5 * sin(wobble(gp + vec2(3.1, 1.7), time * 0.4) * 6.2831 - time * 0.5);
        mote = pow(max(n1, n2), 5.0) * 0.35;
    }

    vec3 final = field.rgb * bright + vec3(mote, mote * 0.82, mote * 0.6);
    if (has_bubbles == 1) {
        // Push liquid color away from gray (more saturated) and add a touch more
        // brightness — the vivid cartoony water/magma look.
        float lum = dot(final, vec3(0.299, 0.587, 0.114));
        final = mix(vec3(lum), final, 1.30) * 1.15;
    }

    float alpha = field.a * clamp(0.94 + roil * 0.12, 0.0, 1.0);
    color = vec4(final, alpha);
}
";

/// Context for the field-texture substance shader: the compiled program,
/// uniform locations, and a reusable region-quad vertex buffer. The CPU builds a
/// small RGBA field texture (one texel per visible tile) each frame; this shader
/// draws the whole region through it via a multi-tap gaussian blur of the field
/// (which melts the blocky per-tile steps into soft gooey contours) plus a wavy
/// surface displacement and cartoon roil/bubbles/motes.
pub struct FieldShader {
    pub program: u32,
    pub time: i32,
    pub field_tex: i32,
    pub field_size: i32,
    pub has_bubbles: i32,
    pub cell_size: i32,
    pub tile_origin: i32,
    pub transform_matrix: i32,
    pub rect_vertex_buffer: VertexBuffer,
}

impl FieldShader {
    /// Compiles the field shader and fetches uniform locations.
    pub(crate) fn new() -> Result<Self> {
        let program = compile_shader(FIELD_VERTEX_SHADER, FIELD_FRAGMENT_SHADER)?;

        let time = unsafe { gl::GetUniformLocation(program, c"time".as_ptr().cast::<i8>()) };
        let field_tex = unsafe { gl::GetUniformLocation(program, c"field_tex".as_ptr().cast::<i8>()) };
        let field_size = unsafe { gl::GetUniformLocation(program, c"field_size".as_ptr().cast::<i8>()) };
        let has_bubbles = unsafe { gl::GetUniformLocation(program, c"has_bubbles".as_ptr().cast::<i8>()) };
        let cell_size = unsafe { gl::GetUniformLocation(program, c"cell_size".as_ptr().cast::<i8>()) };
        let tile_origin = unsafe { gl::GetUniformLocation(program, c"tile_origin".as_ptr().cast::<i8>()) };
        let transform_matrix = unsafe { gl::GetUniformLocation(program, c"transform_matrix".as_ptr().cast::<i8>()) };

        // The region quad is re-filled each frame by the caller (Screen rect set
        // with world-coords in the tex channel); the buffer object is created
        // here so a single VAO/VBO/EBO is reused across frames.
        let rect_vertex_buffer = VertexBuffer::new();

        Ok(Self { program, time, field_tex, field_size, has_bubbles, cell_size, tile_origin, transform_matrix, rect_vertex_buffer })
    }
}
