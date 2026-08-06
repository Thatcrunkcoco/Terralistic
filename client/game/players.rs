use anyhow::{anyhow, Result};
use hecs::Entity;

use crate::client::game::camera::Camera;
use crate::client::game::networking::{ClientNetworking, WelcomePacketEvent};
use crate::libraries::events::Event;
use crate::libraries::graphics as gfx;
use crate::shared::blocks::{Blocks, BLOCK_WIDTH, RENDER_BLOCK_WIDTH, RENDER_SCALE};
use crate::shared::entities::{Entities, EntityDespawnEvent, HealthComponent, PhysicsComponent, PositionComponent};
use super::player_body::draw_player;
use crate::shared::mod_manager::ModManager;
use crate::shared::packet::Packet;
use crate::shared::players::{
    spawn_player, update_players_ms, Direction, MovingType, PlayerComponent, PlayerMovingPacketToClient, PlayerMovingPacketToServer, PlayerSpawnPacket, PLAYER_HEIGHT, PLAYER_MAX_HEALTH, PLAYER_WIDTH,
};

pub struct ClientPlayers {
    main_player: Option<Entity>,
    main_player_name: String,
    player_texture: gfx::Texture,
    player_colors: super::player_body::PlayerColors,
    procedural_player: bool,
    animation_start: std::time::Instant,
    waiting_for_player: bool,
    pub controls_enabled: bool,
}

impl ClientPlayers {
    pub fn new(player_name: &str) -> Self {
        Self {
            main_player: None,
            main_player_name: player_name.to_owned(),
            player_texture: gfx::Texture::new(),
            player_colors: super::player_body::PlayerColors::fallback(),
            procedural_player: true,
            animation_start: std::time::Instant::now(),
            controls_enabled: true,
            waiting_for_player: true,
        }
    }

    pub fn load_resources(&mut self, mods: &ModManager) -> Result<()> {
        let mut template_surface = gfx::Surface::deserialize_from_bytes(
            mods.get_resource("misc:skin_template.opa")
                .ok_or_else(|| anyhow::anyhow!("Failed to load misc:skin_template.opa from mod manager"))?,
        )?;

        let player_surface = gfx::Surface::deserialize_from_bytes(mods.get_resource("misc:skin.opa").ok_or_else(|| anyhow::anyhow!("Failed to load misc:skin.opa from mod manager"))?)?;

        for (_, color) in template_surface.iter_mut() {
            let x = color.r as i32 / 8;
            let y = color.g as i32 / 8;

            *color = *player_surface.get_pixel(gfx::IntPos(x, y))?;
        }

        // Sample the flat palette colors that drive the procedural paper-doll
        // body from the (already palette-remapped) standing frame.
        self.player_colors = super::player_body::sample_colors(&template_surface);

        self.player_texture = gfx::Texture::load_from_surface(&template_surface);

        Ok(())
    }

    fn send_moving_state(networking: &mut ClientNetworking, player_component: &PlayerComponent) -> Result<()> {
        let packet = Packet::new(PlayerMovingPacketToServer {
            moving_type: player_component.get_moving_type(),
            jumping: player_component.jumping,
        })?;

        networking.send_packet(packet)?;
        Ok(())
    }

    fn set_jumping(networking: &mut ClientNetworking, player_component: &mut PlayerComponent, jumping: bool) -> Result<()> {
        if jumping == player_component.jumping {
            return Ok(());
        }

        player_component.jumping = jumping;

        Self::send_moving_state(networking, player_component)?;

        Ok(())
    }

    fn set_moving_type(networking: &mut ClientNetworking, moving_type: MovingType, player_component: &mut PlayerComponent, physics: &mut PhysicsComponent) -> Result<()> {
        if moving_type == player_component.get_moving_type() {
            return Ok(());
        }

        player_component.set_moving_type(moving_type, physics);
        Self::send_moving_state(networking, player_component)?;

        Ok(())
    }

    pub fn update(&self, graphics: &gfx::GraphicsContext, entities: &mut Entities, networking: &mut ClientNetworking, blocks: &Blocks) -> Result<()> {
        if let Some(main_player) = self.main_player {
            if let Ok((physics, player_component)) = entities.ecs.query_one_mut::<(&mut PhysicsComponent, &mut PlayerComponent)>(main_player) {
                Self::set_jumping(networking, player_component, graphics.get_key_state(gfx::Key::Space) && self.controls_enabled)?;

                let key_a_pressed = graphics.get_key_state(gfx::Key::A) && self.controls_enabled;
                let key_d_pressed = graphics.get_key_state(gfx::Key::D) && self.controls_enabled;

                let moving_type = match (key_a_pressed, key_d_pressed) {
                    (true, false) => MovingType::MovingLeft,
                    (false, true) => MovingType::MovingRight,
                    _ => MovingType::Standing,
                };

                Self::set_moving_type(networking, moving_type, player_component, physics)?;
            }
        }

        update_players_ms(entities, blocks);

        Ok(())
    }

    pub fn render(&self, graphics: &gfx::GraphicsContext, entities: &mut Entities, camera: &Camera) {
        let anim_time = self.animation_start.elapsed().as_secs_f32();
        for (_, (position, player_component, physics)) in entities.ecs.query_mut::<(&PositionComponent, &PlayerComponent, &PhysicsComponent)>() {
            let top_x = position.x() * RENDER_BLOCK_WIDTH - camera.get_top_left(graphics).0 * RENDER_BLOCK_WIDTH;
            let top_y = position.y() * RENDER_BLOCK_WIDTH - camera.get_top_left(graphics).1 * RENDER_BLOCK_WIDTH;

            let flipped = match player_component.direction {
                Direction::Left => true,
                Direction::Right => false,
            };

            if self.procedural_player {
                // Feet sit at the bottom of the player's collision box
                // (position.y + PLAYER_HEIGHT blocks), on the ground.
                let feet = gfx::FloatPos(
                    top_x + PLAYER_WIDTH * RENDER_BLOCK_WIDTH * 0.5,
                    top_y + PLAYER_HEIGHT * RENDER_BLOCK_WIDTH,
                );
                draw_player(
                    graphics,
                    feet,
                    physics.velocity_x,
                    physics.velocity_y,
                    anim_time,
                    flipped,
                    &self.player_colors,
                );
            } else {
                let src_rect = gfx::Rect::new(
                    gfx::FloatPos(player_component.animation_frame as f32 * PLAYER_WIDTH * BLOCK_WIDTH, 0.0),
                    gfx::FloatSize(PLAYER_WIDTH * BLOCK_WIDTH, PLAYER_HEIGHT * BLOCK_WIDTH),
                );
                self.player_texture.render(graphics, RENDER_SCALE, gfx::FloatPos(top_x.round(), top_y.round()), Some(src_rect), flipped, None);
            }
        }
    }

    pub fn on_event(&mut self, event: &Event, entities: &mut Entities) -> Result<()> {
        if let Some(packet_event) = event.downcast::<WelcomePacketEvent>() {
            let packet = &packet_event.packet;
            if let Some(packet) = packet.try_deserialize::<PlayerSpawnPacket>() {
                let player = spawn_player(entities, packet.x, packet.y, &packet.name, packet.id, HealthComponent::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH))?;
                if packet.name == self.main_player_name {
                    self.main_player = Some(player);
                    self.waiting_for_player = false;
                }
            }
        }

        if let Some(packet_event) = event.downcast::<Packet>() {
            if let Some(packet) = packet_event.try_deserialize::<PlayerSpawnPacket>() {
                let player = spawn_player(entities, packet.x, packet.y, &packet.name, packet.id, HealthComponent::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH))?;
                if packet.name == self.main_player_name {
                    self.main_player = Some(player);
                    self.waiting_for_player = false;
                }
            } else if let Some(packet) = packet_event.try_deserialize::<PlayerMovingPacketToClient>() {
                let entity = entities.get_entity_from_id(packet.player_id)?;
                let mut physics_component = entities.ecs.query_one::<&mut PhysicsComponent>(entity)?.get().ok_or_else(|| anyhow!("unwrap failed"))?.clone();
                {
                    let player_component = entities.ecs.query_one_mut::<&mut PlayerComponent>(entity)?;
                    player_component.set_moving_type(packet.moving_type, &mut physics_component);
                    player_component.jumping = packet.jumping;
                }

                entities.ecs.insert_one(entity, physics_component)?;
            }
        }

        if let Some(despawn_event) = event.downcast::<EntityDespawnEvent>() {
            if let Some(main_player) = self.main_player {
                let id = entities.get_id_from_entity(main_player)?;
                if id == despawn_event.id {
                    self.main_player = None;
                }
            }
        }

        Ok(())
    }

    pub const fn get_main_player(&self) -> Option<Entity> {
        self.main_player
    }

    /// Toggles between the procedural paper-doll body and the original
    /// sprite-sheet animation (useful for A/B comparison). Intended as public
    /// API for an in-game toggle; not yet bound to the debug menu.
    #[allow(dead_code)]
    pub fn set_procedural_player(&mut self, enabled: bool) {
        self.procedural_player = enabled;
    }

    #[allow(dead_code)]
    pub const fn procedural_player(&self) -> bool {
        self.procedural_player
    }

    pub const fn is_waiting_for_player(&self) -> bool {
        self.waiting_for_player
    }
}
