//! Procedural "paper-doll" player renderer.
//!
//! Replaces the low-res Terraria-style 16-frame walk-sheet with an eased,
//! cartoony skeleton built from rounded limb capsules (head / torso / 2 arms /
//! 2 legs), the way ONI / Cult of the Lamb / Paper Mario style their characters.
//!
//! The body is drawn with a **slice-based rounded capsule** so limbs read as
//! soft pills rather than hard rectangles. Each limb first gets a slightly
//! larger dark *outline* capsule and then the fill on top, giving every limb a
//! chunky ink border that matches the sprite's original black outlines.
//!
//! Animation is continuous (not a frame counter): a walk `phase` sweeps with
//! horizontal speed so arms/legs swing smoothly (sine, opposite phase), plus a
//! subtle idle bob and a squash & stretch driven by vertical velocity.
//!
//! Colors come from the already palette-remapped `skin.opa` (skin / hair /
//! shirt / pants sampled from the standing frame of the template), so custom
//! player skins carry through to the procedural body.

use crate::libraries::graphics as gfx;

/// Four flat palette colors lifted from the remapped skin sprite, plus the ink
/// outline color used to border every limb capsule.
#[derive(Clone, Copy)]
pub struct PlayerColors {
    pub skin: gfx::Color,
    pub hair: gfx::Color,
    pub shirt: gfx::Color,
    pub pants: gfx::Color,
    pub outline: gfx::Color,
}

impl PlayerColors {
    #[must_use]
    pub fn fallback() -> Self {
        Self {
            skin: gfx::Color::new(214, 182, 46, 255),
            hair: gfx::Color::new(99, 79, 38, 255),
            shirt: gfx::Color::new(56, 178, 0, 255),
            pants: gfx::Color::new(53, 141, 231, 255),
            outline: gfx::Color::new(8, 8, 12, 255),
        }
    }
}

// ---- tuning constants (source-px feet-origin, +up) ----
const AMP: f32 = 7.0; // walk swing amplitude (horizontal px)
const LIFT: f32 = 5.0; // foot lift while swinging
const BOB: f32 = 1.6; // idle bob amplitude
const OUTLINE: f32 = 2.2; // ink border thickness around each limb
const SLICES: usize = 6; // slices used to approximate a rounded capsule

/// Sample a player's flat palette colors from the remapped template surface by
/// majority color within the standing frame's fixed semantic regions.
pub fn sample_colors(template: &gfx::Surface) -> PlayerColors {
    // Standing frame starts at source column 16; each frame is 16x24 source px.
    const FW: i32 = 16;

    let mode = |x0: i32, x1: i32, y0: i32, y1: i32| -> Option<gfx::Color> {
        let mut counts: std::collections::HashMap<gfx::Color, u32> = Default::default();
        for y in y0..y1 {
            for x in x0..x1 {
                let c = match template.get_pixel(gfx::IntPos(16 + x, y)) {
                    Ok(c) => *c,
                    Err(_) => continue,
                };
                let is_black = c.r < 40 && c.g < 40 && c.b < 40;
                if c.a > 128 && !is_black {
                    *counts.entry(c).or_insert(0) += 1;
                }
            }
        }
        counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c)
    };

    let fallback = PlayerColors::fallback();
    let skin = mode(FW / 2 - 2, FW / 2 + 2, 6, 11).unwrap_or(fallback.skin);
    let hair = mode(2, FW - 2, 0, 6).unwrap_or(fallback.hair);
    let shirt = mode(2, FW - 2, 12, 18).unwrap_or(fallback.shirt);
    let pants = mode(2, FW - 2, 18, 24).unwrap_or(fallback.pants);

    PlayerColors { skin, hair, shirt, pants, outline: fallback.outline }
}

/// Draws one filled vertical capsule between two points, in **screen space**
/// (already transformed), as a stack of horizontal slices that taper towards
/// rounded end caps.
fn add_capsule_fill(rects: &mut gfx::RectArray, cx: f32, y0: f32, y1: f32, half_w: f32, color: gfx::Color) {
    let tex = gfx::Rect::new(gfx::FloatPos(0.0, 0.0), gfx::FloatSize(1.0, 1.0));
    let (top, bottom) = if y0 < y1 { (y0, y1) } else { (y1, y0) };
    let height = bottom - top;
    if height <= 0.0 {
        return;
    }
    for i in 0..SLICES {
        let t0 = i as f32 / SLICES as f32;
        let t1 = (i + 1) as f32 / SLICES as f32;
        let y_lo = top + height * t0;
        let y_hi = top + height * t1;
        let mid = (t0 + t1) * 0.5;
        let d = (mid * 2.0 - 1.0).abs(); // 0 = centre, 1 = end cap
        let flat = 0.55;
        let hw = if d < flat {
            half_w
        } else {
            let c = (d - flat) / (1.0 - flat);
            half_w * (1.0 - c * c).max(0.0).sqrt()
        };
        let rect = gfx::Rect::new(gfx::FloatPos(cx - hw, y_lo), gfx::FloatSize(hw * 2.0, y_hi - y_lo));
        rects.add_rect(&rect, &[color; 4], &tex);
    }
}

/// Draws a rounded limb (in *visual* coordinates where `cx`/`y` are already in
/// screen px) with a chunky ink outline behind the fill.
fn add_capsule(rects: &mut gfx::RectArray, cx: f32, y_centre: f32, half_h: f32, half_w: f32, color: gfx::Color, outline: gfx::Color) {
    // outline first (a bit bigger), then fill
    add_capsule_fill(rects, cx, y_centre - half_h - OUTLINE, y_centre + half_h + OUTLINE, half_w + OUTLINE, outline);
    add_capsule_fill(rects, cx, y_centre - half_h, y_centre + half_h, half_w, color);
}

/// Render one player as a procedural cartoon body.
///
/// `feet` is the screen position of the player's feet centre (feet on ground).
/// `vel_x` / `vel_y` drive the walk phase and squash & stretch. `flipped`
/// mirrors the body horizontally. `colors` holds the remapped skin palette.
pub fn draw_player(
    graphics: &gfx::GraphicsContext,
    feet: gfx::FloatPos,
    vel_x: f32,
    vel_y: f32,
    time: f32,
    flipped: bool,
    colors: &PlayerColors,
) {
    let mut rects = gfx::RectArray::new();

    let fx = feet.0;
    let fy = feet.1; // feet on ground (bottom of body)

    // ---- animation parameters ----
    let speed = vel_x.abs();
    let moving = speed > 0.4;

    // Squash & stretch from vertical velocity (rise -> stretch tall/ thin).
    let rising = (-vel_y).max(0.0);
    let stretch = 1.0 + (rising * 0.06).min(0.30);
    // horizontal squash is the inverse (preserve-ish area)
    let hscale = 1.0 / stretch;
    let vs = stretch;

    // Walk phase sweeps with horizontal speed; standing -> rest pose.
    let phase = time * speed * 1.4;
    let swing = if moving { phase.sin() } else { 0.0 };
    // idle bob
    let bob = if moving { 0.0 } else { BOB * time.sin() };

    // local -> screen: x is centred at fx (mirror via flip), y grows *up*
    // (negative screen), scaled by squash/stretch and offset by idle bob.
    let sign: f32 = if flipped { -1.0 } else { 1.0 };
    let to_x = |dx: f32| fx + dx * sign * hscale;
    let to_y = |up: f32| fy - (up * vs + bob);

    // ---- body parts (in "up" px above feet, dx px from centre) ----

    // Legs (pants) from the hips down to the feet, swinging opposite to the
    // arms in a proper alternating gait. `sgn` picks left (-1) / right (+1).
    let leg_base = 5.5;
    let leg_amp = if moving { AMP * 0.75 } else { 0.0 };
    let hip_up = 14.0;
    for (_, sgn) in [(0.0, -1.0), (1.0, 1.0)] {
        // horizontal step offset (forward/back) for this leg
        let step = leg_amp * (swing * sgn);
        // the leading foot lifts off the ground while it steps forward
        let forward = (swing * sgn).max(0.0);
        let lift = if moving { LIFT * forward } else { 0.0 };
        let leg_cx = leg_base * sgn + step;
        let cx = to_x(leg_cx);
        let y_top = to_y(hip_up);
        let y_bot = to_y(lift);
        add_capsule(&mut rects, cx, (y_top + y_bot) * 0.5, (y_top - y_bot).abs() * 0.5, 3.2, colors.pants, colors.outline);
    }

    // Torso (shirt): a rounded body from the hips up to the shoulders.
    let torso_hw = 11.0;
    let shoulder_up = 27.0;
    let torso_y = to_y((hip_up + shoulder_up) * 0.5);
    let torso_hh = (to_y(hip_up) - to_y(shoulder_up)).abs() * 0.5;
    add_capsule(&mut rects, to_x(0.0), torso_y, torso_hh * 1.02, torso_hw * 0.92, colors.shirt, colors.outline);
    // a small chest/collar pad so the torso meets the head without a gap
    add_capsule_fill(&mut rects, to_x(0.0), to_y(shoulder_up - 1.5), to_y(shoulder_up + 1.5), torso_hw * 0.6, colors.shirt);

    // Arms (sleeves) hanging from the shoulders, swinging opposite to the legs.
    let arm_amp = if moving { AMP * 0.6 } else { 0.0 };
    let arm_top = 26.0;
    let arm_bot = 13.0;
    for (_, sgn) in [(0.0, -1.0), (1.0, 1.0)] {
        let sw = arm_amp * (swing * -sgn);
        let arm_cx = 10.0 * sgn + sw;
        let cx = to_x(arm_cx);
        let y_top = to_y(arm_top);
        let y_bot = to_y(arm_bot);
        add_capsule(&mut rects, cx, (y_top + y_bot) * 0.5, (y_top - y_bot).abs() * 0.5, 2.6, colors.shirt, colors.outline);
    }

    // Head (skin): a big rounded head on top of the shoulders.
    let head_bot = 24.0;
    let head_top = 47.0;
    let head_cx = to_x(0.0);
    let head_y = to_y((head_bot + head_top) * 0.5);
    let head_hh = (to_y(head_bot) - to_y(head_top)).abs() * 0.5;
    add_capsule(&mut rects, head_cx, head_y, head_hh * 1.02, 13.0, colors.skin, colors.outline);

    // Hair cap: a rounded brown cap over the top of the head.
    let hair_bot = 36.0;
    let hair_top = head_top + 1.0;
    add_capsule_fill(&mut rects, head_cx, to_y(hair_bot), to_y(hair_top), 13.0, colors.hair);

    // ---- present ----
    // Push the freshly-built CPU vertices to the GPU before drawing.
    rects.update();
    rects.render(graphics, None, gfx::FloatPos(0.0, 0.0));
}
