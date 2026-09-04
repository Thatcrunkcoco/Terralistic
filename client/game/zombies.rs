use anyhow::{anyhow, Result};

use crate::client::game::camera::Camera;
use crate::libraries::events::Event;
use crate::libraries::graphics as gfx;
use crate::shared::blocks::{RENDER_BLOCK_WIDTH, RENDER_SCALE};
use crate::shared::entities::{Entities, EntityKind, EntitySpawnPacket, PositionComponent, PhysicsComponent, ZombieComponent};
use crate::shared::mod_manager::ModManager;
use crate::shared::packet::Packet;

/// Number of procedural walk frames baked into the zombie sprite sheet.
pub const ZOMBIE_WALK_FRAMES: usize = 16;

/// Seconds per full walk cycle (16 frames). Drives both the frame pacing and
/// the body bob so limbs and bounce stay in lockstep.
const WALK_CYCLE_SECONDS: f32 = 1.0;
/// Pixel size of a single walk frame (matches the player's 2x3 block body).
const FRAME_PX_W: u32 = 16;
const FRAME_PX_H: u32 = 24;

/// Sub-pixel subdivision per axis used by [`fill`]: each pixel is sampled at
/// `SUBPIXELS` x `SUBPIXELS` points, so fractional rect coordinates keep their
/// coverage instead of snapping to whole pixels. Higher = smoother limb motion
/// in the baked sprite at slightly higher bake cost (bake runs once at load).
const SUBPIXELS: usize = 4;

/// Palette used by the procedural zombie.
const SKIN: gfx::Color = gfx::Color::new(94, 158, 70, 255); // sickly green
const SHIRT: gfx::Color = gfx::Color::new(70, 84, 64, 255); // dark olive
const PANT: gfx::Color = gfx::Color::new(48, 58, 52, 255); // dark trousers
const OUTLINE: gfx::Color = gfx::Color::new(24, 40, 22, 255); // dark outline

/// Renders every zombie in the world with a procedurally animated walk cycle.
///
/// The zombie "art" is generated entirely in code (no PNGs), which keeps
/// artistic input near-zero and makes the sprite deterministic and testable:
/// the same frame index always produces the same pixels. Animation is driven by
/// each zombie's own `animation_progress` so it works even without networked
/// per-entity animation state.
pub struct ClientZombies {
    sprite_sheet: gfx::Texture,
}

impl ClientZombies {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sprite_sheet: gfx::Texture::new(),
        }
    }

    /// Builds the procedural zombie sprite sheet from code. No mod resources are
    /// needed. Called once when the client initializes the game view.
    pub fn load_resources(&mut self, _mods: &ModManager) -> Result<()> {
        let surface = build_zombie_sheet(ZOMBIE_WALK_FRAMES)?;
        self.sprite_sheet = gfx::Texture::load_from_surface(&surface);
        Ok(())
    }

    /// Handles the server's entity-spawn packet so a zombie is created locally.
    pub fn on_event(&mut self, event: &Event, entities: &mut Entities) -> Result<()> {
        if let Some(packet) = event.downcast::<Packet>() {
            if let Some(spawn) = packet.try_deserialize::<EntitySpawnPacket>() {
                if spawn.kind == EntityKind::Zombie {
                    let _ent = crate::shared::entities::spawn_zombie(entities, spawn.x, spawn.y)?;
                }
            }
        }
        Ok(())
    }

    /// Advances zombie animation timers using the same fixed timestep cadence
    /// the server uses (called on the 5ms sub-tick), so the client-side walk
    /// cycle stays roughly in sync with the server's movement. Also lerps each
    /// zombie's position toward the server snapshot target so their bodies
    /// glide between the (1 Hz) authoritative syncs instead of snapping.
    pub fn update(&self, entities: &mut Entities) -> Result<()> {
        for (_, (position, zombie)) in entities.ecs.query_mut::<(&mut PositionComponent, &mut ZombieComponent)>() {
            zombie.animation_progress += 0.005;
            // Keep it bounded so the animation stays continuous.
            if zombie.animation_progress > 10_000.0 {
                zombie.animation_progress = 0.0;
            }

            // Smooth the authoritative position correction: move a fraction of
            // the way to the target each subtick. This absorbs the 1 Hz sync
            // step while eventually converging to the server's position.
            const LERP: f32 = 0.15;
            position.set_x(position.x() + (zombie.target_x() - position.x()) * LERP);
            position.set_y(position.y() + (zombie.target_y() - position.y()) * LERP);
        }
        Ok(())
    }

    /// Converts wall-clock animation time into the baked-sheet frame index,
    /// paced by [`WALK_CYCLE_SECONDS`]. Splits the cycle into frame-sized
    /// phases and picks the frame whose phase window contains the current
    /// time, cycling cleanly across the frames.
    fn frame_index(animation_progress: f32) -> usize {
        let cycle_len = WALK_CYCLE_SECONDS / ZOMBIE_WALK_FRAMES as f32;
        let raw = animation_progress / cycle_len;
        raw.floor() as usize % ZOMBIE_WALK_FRAMES
    }

    /// Draws every zombie. Facing is derived from the synced horizontal velocity;
    /// the walk frame comes from the zombie's animation timer.
    pub fn render(&self, graphics: &gfx::GraphicsContext, camera: &Camera, entities: &mut Entities) -> Result<()> {
        let top_left = camera.get_top_left(graphics);
        for (_, (position, physics, zombie)) in entities.ecs.query_mut::<(&PositionComponent, &PhysicsComponent, &ZombieComponent)>() {
            // Derive facing from velocity so direction stays server-authoritative.
            let flipped = physics.velocity_x < -0.01;

            // Keep the frame within [0, ZOMBIE_WALK_FRAMES), paced by the walk
            // cycle so the full set of frames plays once per WALK_CYCLE_SECONDS.
            let frame = Self::frame_index(zombie.animation_progress);

            // Choose the src rect for this frame in the horizontal strip.
            let src_rect = gfx::Rect::new(
                gfx::FloatPos(frame as f32 * FRAME_PX_W as f32, 0.0),
                gfx::FloatSize(FRAME_PX_W as f32, FRAME_PX_H as f32),
            );

            // Center the sprite on its collision box. The sprite fills the box
            // exactly (frame is 2x3 blocks at RENDER_SCALE), so no extra offset.
            let screen_x = position.x() * RENDER_BLOCK_WIDTH - top_left.0 * RENDER_BLOCK_WIDTH;
            let screen_y = position.y() * RENDER_BLOCK_WIDTH - top_left.1 * RENDER_BLOCK_WIDTH;

            self.sprite_sheet.render(graphics, RENDER_SCALE, gfx::FloatPos(screen_x, screen_y), Some(src_rect), flipped, None);
        }
        Ok(())
    }
}

/// Builds a sprite sheet of `frame_count` zombie walk frames laid out in a
/// horizontal strip (each frame is FRAME_PX_W x FRAME_PX_H). Every frame is
/// drawn deterministically from the frame index.
pub fn build_zombie_sheet(frame_count: usize) -> Result<gfx::Surface> {
    if frame_count == 0 {
        return Err(anyhow!("need at least one zombie frame"));
    }
    let size = gfx::IntSize(FRAME_PX_W * frame_count as u32, FRAME_PX_H);
    let mut sheet = gfx::Surface::new(size);
    for frame in 0..frame_count {
        let origin = gfx::IntPos(frame as i32 * FRAME_PX_W as i32, 0);
        draw_zombie_frame(&mut sheet, origin, frame);
    }
    Ok(sheet)
}

/// Draws a single zombie body into `sheet` at pixel `origin` for the given
/// walk-frame index. Body segments are placed via helper `limb`.
fn draw_zombie_frame(sheet: &mut gfx::Surface, origin: gfx::IntPos, frame: usize) {
    let of = gfx::FloatPos(origin.0 as f32, origin.1 as f32);
    let sway = arc_sway(frame);

    // Body bob: the torso/head rise and fall once per step (twice per cycle)
    // so the walk reads as a bounce, not a rigid slide. Feet stay planted.
    let bob = arc_sway(frame);
    let bob_offset = -bob.abs() * 1.0;

    // Head: a 6x6 green block at the top center.
    fill(sheet, of + gfx::FloatPos(5.0, 1.0 + bob_offset), 6, 7, SKIN);
    // Eyes/gape: two dark pits.
    fill(sheet, of + gfx::FloatPos(6.0, 3.0 + bob_offset), 1, 1, OUTLINE);
    fill(sheet, of + gfx::FloatPos(9.0, 3.0 + bob_offset), 1, 1, OUTLINE);

    // Torso under head.
    fill(sheet, of + gfx::FloatPos(6.0, 8.0 + bob_offset), 4, 5, SHIRT);

    // Arms hang at the sides at FIXED x and pump vertically, in opposite
    // phases (left leg forward => right arm forward and vice versa), so they
    // read as stride motion while staying attached to the torso. Fractional
    // offsets keep the motion continuous across the 16-frame cycle.
    let arm_pump = sway * 1.5;
    fill(sheet, of + gfx::FloatPos(3.0, 9.0 + bob_offset - arm_pump), 2, 5, SHIRT);
    fill(sheet, of + gfx::FloatPos(11.0, 9.0 + bob_offset + arm_pump), 2, 5, SHIRT);

    // Legs: front/back alternate with the frame to read as walking. The
    // amplitude is kept at 2px so the leg tops stay anchored under the torso
    // (spans x 6..10) instead of splaying out of the body silhouette.
    let leg_swing = sway * 2.0;
    // Left leg (behind).
    let left_leg_x = 5.0 + leg_swing / 2.0;
    // Right leg (in front).
    let right_leg_x = 9.0 - leg_swing / 2.0;
    fill(sheet, of + gfx::FloatPos(left_leg_x, 13.0), 2, 8, PANT);
    fill(sheet, of + gfx::FloatPos(right_leg_x, 13.0), 2, 8, PANT);

    // Feet: small dark blocks at the bottom, anchored to their leg so the
    // foot never slides out from under the leg column (foot is 3px wide and
    // centered on the leg's 2px column).
    fill(sheet, of + gfx::FloatPos(left_leg_x - 0.5, 21.0), 3, 2, OUTLINE);
    fill(sheet, of + gfx::FloatPos(right_leg_x - 0.5, 21.0), 3, 2, OUTLINE);
}

/// Maps a frame index to a smooth swing value in [-1, 1] over the walk cycle.
fn arc_sway(frame: usize) -> f32 {
    // Even frames push one way, odd frames the other, with a sine-like edge so
    // the limbs don't snap. Deterministic for a given frame.
    let t = (frame as f32 / ZOMBIE_WALK_FRAMES as f32) * std::f32::consts::TAU;
    t.sin()
}

/// Fills a solid rectangle of `color` into `sheet` starting at `top_left`,
/// with fractional (sub-pixel) coordinates and dimensions. Each pixel's
/// coverage is estimated by sampling `SUBPIXELS` x `SUBPIXELS` points so a
/// rect offset by a fraction of a pixel renders its true coverage instead of
/// snapping to whole pixels.
fn fill(sheet: &mut gfx::Surface, top_left: gfx::FloatPos, w: i32, h: i32, color: gfx::Color) {
    let min_x = top_left.0;
    let min_y = top_left.1;
    let max_x = min_x + w as f32;
    let max_y = min_y + h as f32;

    // Pixel index range touched by this rect (clamped into bounds below).
    let px0 = min_x.floor() as i32;
    let py0 = min_y.floor() as i32;
    let px1 = max_x.ceil() as i32;
    let py1 = max_y.ceil() as i32;

    let samples = SUBPIXELS as f32;
    let step = 1.0 / samples;

    for py in py0..py1 {
        for px in px0..px1 {
            // get_pixel_mut below bounds-checks writes; out-of-range pixels are
            // silently skipped, so fractional rects clipped by the frame edge
            // still bake correctly.
            let mut covered = 0;
            for sy in 0..SUBPIXELS {
                for sx in 0..SUBPIXELS {
                    let sample_x = px as f32 + (sx as f32 + 0.5) * step;
                    let sample_y = py as f32 + (sy as f32 + 0.5) * step;
                    if sample_x >= min_x && sample_x < max_x && sample_y >= min_y && sample_y < max_y {
                        covered += 1;
                    }
                }
            }

            if covered == 0 {
                continue;
            }

            if let Ok(px_ref) = sheet.get_pixel_mut(gfx::IntPos(px, py)) {
                if covered == SUBPIXELS * SUBPIXELS {
                    *px_ref = color;
                } else {
                    // Straight-alpha compositing: partial coverage keeps the
                    // limb's own RGB (no darkening toward the underneath
                    // pixel's channels) and scales only the alpha, so the
                    // soft edge stays the limb color and composites cleanly
                    // over any background at draw time.
                    let coverage = covered as f32 / (SUBPIXELS * SUBPIXELS) as f32;
                    let src_a = f32::from(color.a) / 255.0 * coverage;
                    let dst = *px_ref;
                    let dst_a = f32::from(dst.a) / 255.0;
                    // "over" operator: result = src over dst, un-premultiplied.
                    let out_a = src_a + dst_a * (1.0 - src_a);
                    let blend_channel = |src: u8, dst: u8| -> u8 {
                        if out_a <= 0.0 {
                            0
                        } else {
                            ((f32::from(src) / 255.0 * src_a + f32::from(dst) / 255.0 * dst_a * (1.0 - src_a)) / out_a * 255.0) as u8
                        }
                    };
                    *px_ref = gfx::Color::new(
                        blend_channel(color.r, dst.r),
                        blend_channel(color.g, dst.g),
                        blend_channel(color.b, dst.b),
                        (out_a * 255.0) as u8,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_sizes_match_frame_count() {
        let sheet = build_zombie_sheet(16).unwrap();
        assert_eq!(sheet.get_size(), gfx::IntSize(256, 24));
    }

    #[test]
    fn anchoring_corners_are_transparent() {
        let sheet = build_zombie_sheet(16).unwrap();
        // Top-left and top-right of the first frame should be clear background.
        assert_eq!(*sheet.get_pixel(gfx::IntPos(0, 0)).unwrap(), gfx::Color::new(0, 0, 0, 0));
        assert_eq!(*sheet.get_pixel(gfx::IntPos(15, 0)).unwrap(), gfx::Color::new(0, 0, 0, 0));
    }

    #[test]
    fn head_is_present_in_frame_zero() {
        let sheet = build_zombie_sheet(16).unwrap();
        assert_eq!(*sheet.get_pixel(gfx::IntPos(5, 1)).unwrap(), SKIN);
    }

    /// Visual-inspection hook for the agent / devs: run with
    /// `CAPTURE_SHEET=<dir> cargo test dump_zombie_sheet -- --nocapture` to
    /// write the full baked walk sheet as a zero-dependency PPM (P6) image at
    /// 8x nearest-neighbor zoom over a white background. The agent can then
    /// read and review every frame directly.
    #[test]
    fn dump_zombie_sheet_when_requested() {
        let Ok(dir) = std::env::var("CAPTURE_SHEET") else { return };
        let sheet = build_zombie_sheet(ZOMBIE_WALK_FRAMES).unwrap();
        let scale = 8u32;
        let size = sheet.get_size();
        let (out_w, out_h) = (size.0 * scale, size.1 * scale);
        let mut png = vec![0u8; (out_w * out_h * 3) as usize];
        for y in 0..out_h {
            for x in 0..out_w {
                let src = sheet.get_pixel(gfx::IntPos((x / scale) as i32, (y / scale) as i32)).unwrap();
                // sin over the cycle is zero at both ends (frame 0 == frame N),
                // so transparency should only exist at the strip ends.
                let a = f32::from(src.a) / 255.0;
                let blend = |channel: u8| (f32::from(channel) * a + 255.0 * (1.0 - a)) as u8;
                let idx = ((y * out_w + x) * 3) as usize;
                png[idx] = blend(src.r);
                png[idx + 1] = blend(src.g);
                png[idx + 2] = blend(src.b);
            }
        }
        let header = format!("P6\n{out_w} {out_h}\n255\n");
        let path = std::path::Path::new(&dir).join("zombie_sheet.ppm");
        std::fs::write(path.clone(), header.as_bytes()).unwrap();
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        std::io::Write::write_all(&mut file, &png).unwrap();
        println!("zombie sheet dumped to {}/zombie_sheet.ppm", dir);
    }
}
