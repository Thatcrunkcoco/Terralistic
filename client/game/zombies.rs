use anyhow::{anyhow, Result};

use crate::client::game::camera::Camera;
use crate::libraries::events::Event;
use crate::libraries::graphics as gfx;
use crate::shared::blocks::{RENDER_BLOCK_WIDTH, RENDER_SCALE};
use crate::shared::entities::{Entities, EntityKind, EntitySpawnPacket, PositionComponent, PhysicsComponent, ZombieComponent};
use crate::shared::mod_manager::ModManager;
use crate::shared::packet::Packet;

/// Number of procedural walk frames baked into the zombie sprite sheet.
pub const ZOMBIE_WALK_FRAMES: usize = 4;
/// Pixel size of a single walk frame (matches the player's 2x3 block body).
const FRAME_PX_W: u32 = 16;
const FRAME_PX_H: u32 = 24;

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
    /// cycle stays roughly in sync with the server's movement.
    pub fn update(&self, entities: &mut Entities) -> Result<()> {
        for (_, zombie) in entities.ecs.query_mut::<&mut ZombieComponent>() {
            zombie.animation_progress += 0.005;
            // Keep it bounded so the animation stays continuous.
            if zombie.animation_progress > 10_000.0 {
                zombie.animation_progress = 0.0;
            }
        }
        Ok(())
    }

    /// Draws every zombie. Facing is derived from the synced horizontal velocity;
    /// the walk frame comes from the zombie's animation timer.
    pub fn render(&self, graphics: &gfx::GraphicsContext, camera: &Camera, entities: &mut Entities) -> Result<()> {
        let top_left = camera.get_top_left(graphics);
        for (_, (position, physics, zombie)) in entities.ecs.query_mut::<(&PositionComponent, &PhysicsComponent, &ZombieComponent)>() {
            // Derive facing from velocity so direction stays server-authoritative.
            let flipped = physics.velocity_x < -0.01;

            // Keep the frame within [0, ZOMBIE_WALK_FRAMES).
            let cycle = ZOMBIE_WALK_FRAMES as f32;
            let raw = zombie.animation_progress % cycle;
            let frame = raw.floor() as usize % ZOMBIE_WALK_FRAMES;

            // Choose the src rect for this frame in the horizontal strip.
            let src_rect = gfx::Rect::new(
                gfx::FloatPos(frame as f32 * FRAME_PX_W as f32, 0.0),
                gfx::FloatSize(FRAME_PX_W as f32, FRAME_PX_H as f32),
            );

            // Center the sprite on its collision box. The sprite fills the box
            // exactly (frame is 2x3 blocks at RENDER_SCALE), so no extra offset.
            let screen_x = (position.x() * RENDER_BLOCK_WIDTH - top_left.0 * RENDER_BLOCK_WIDTH).round();
            let screen_y = (position.y() * RENDER_BLOCK_WIDTH - top_left.1 * RENDER_BLOCK_WIDTH).round();

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
        draw_zombie_frame(&mut sheet, origin, frame, frame_count);
    }
    Ok(sheet)
}

/// Draws a single zombie body into `sheet` at pixel `origin` for the given
/// walk-frame index. Body segments are placed via helper `limb`.
fn draw_zombie_frame(sheet: &mut gfx::Surface, origin: gfx::IntPos, frame: usize, frame_count: usize) {
    // Head: a 6x6 green block at the top center.
    fill(sheet, origin + gfx::IntPos(5, 1), 6, 7, SKIN);
    // Eyes/gape: two dark pits.
    fill(sheet, origin + gfx::IntPos(6, 3), 1, 1, OUTLINE);
    fill(sheet, origin + gfx::IntPos(9, 3), 1, 1, OUTLINE);

    // Torso under head.
    fill(sheet, origin + gfx::IntPos(6, 8), 4, 5, SHIRT);

    // Arms hang at the sides; they sway with the walk cycle.
    let sway = arc_sway(frame, frame_count); // in [-1, 1]
    let arm_lift = (sway * 2.0).round() as i32;
    fill(sheet, origin + gfx::IntPos(3 + arm_lift, 9), 2, 5, SHIRT);
    fill(sheet, origin + gfx::IntPos(11 - arm_lift, 9), 2, 5, SHIRT);

    // Legs: front/back alternate with the frame to read as walking.
    let leg_swing = (sway * 3.0).round() as i32;
    // Left leg (behind).
    fill(sheet, origin + gfx::IntPos(5 + leg_swing / 2, 13), 2, 8, PANT);
    // Right leg (in front).
    fill(sheet, origin + gfx::IntPos(9 - leg_swing / 2, 13), 2, 8, PANT);

    // Feet: small dark blocks at the bottom.
    fill(sheet, origin + gfx::IntPos(4 + leg_swing, 21), 3, 2, OUTLINE);
    fill(sheet, origin + gfx::IntPos(9 - leg_swing, 21), 3, 2, OUTLINE);
}

/// Maps a frame index to a smooth swing value in [-1, 1] over the walk cycle.
fn arc_sway(frame: usize, frame_count: usize) -> f32 {
    // Even frames push one way, odd frames the other, with a sine-like edge so
    // the limbs don't snap. Deterministic for a given (frame, frame_count).
    let t = (frame as f32 / frame_count.max(1) as f32) * std::f32::consts::TAU;
    t.sin()
}

/// Fills a solid rectangle of `color` into `sheet` starting at `top_left`.
fn fill(sheet: &mut gfx::Surface, top_left: gfx::IntPos, w: i32, h: i32, color: gfx::Color) {
    for dy in 0..h {
        for dx in 0..w {
            if let Ok(px) = sheet.get_pixel_mut(top_left + gfx::IntPos(dx, dy)) {
                *px = color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_sizes_match_frame_count() {
        let sheet = build_zombie_sheet(4).unwrap();
        assert_eq!(sheet.get_size(), gfx::IntSize(64, 24));
    }

    #[test]
    fn anchoring_corners_are_transparent() {
        let sheet = build_zombie_sheet(4).unwrap();
        // Top-left and top-right of the first frame should be clear background.
        assert_eq!(*sheet.get_pixel(gfx::IntPos(0, 0)).unwrap(), gfx::Color::new(0, 0, 0, 0));
        assert_eq!(*sheet.get_pixel(gfx::IntPos(15, 0)).unwrap(), gfx::Color::new(0, 0, 0, 0));
    }

    #[test]
    fn head_is_present_in_frame_zero() {
        let sheet = build_zombie_sheet(4).unwrap();
        assert_eq!(*sheet.get_pixel(gfx::IntPos(5, 1)).unwrap(), SKIN);
    }
}
