use anyhow::Result;

use crate::libraries::events::Event;
use crate::server::server_core::networking::{SendTarget, ServerNetworking};
use crate::shared::blocks::Blocks;
use crate::shared::entities::{spawn_zombie, update_zombies_ms, Entities, EntityKind, EntitySpawnPacket, PositionComponent, ZombieComponent, ZOMBIE_HEIGHT};
use crate::shared::packet::Packet;

/// Manages non-player "zombie" entities on the server. Zombies are ordinary
/// entities (position + physics + a `ZombieComponent`) that walk across the
/// world; the server is authoritative for their movement and broadcasts both
/// their spawn and (via the generic entity sync) their position.
pub struct ServerZombies;

impl ServerZombies {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Seeds a handful of zombies near the flat test world's surface, just right
    /// of the demo gas/liquid structures, and broadcasts their spawn to any
    /// already-connected clients. Newly joining clients are told about them via
    /// `on_event` -> `broadcast_spawns`.
    pub fn seed(&self, entities: &mut Entities, blocks: &Blocks, networking: &mut ServerNetworking) -> Result<()> {
        let world_width = blocks.get_size().0 as f32;
        // Two zombies on flat grass to the right of the demo structures (which
        // end around x=290). x=world_width/2 is the player spawn; we offset to
        // the right into open terrain.
        let candidates = [world_width / 2.0 + 60.0, world_width / 2.0 + 68.0];
        for x in candidates {
            let ground_y = find_surface_y(blocks, x);
            let x = x.clamp(1.0, world_width - 1.0);
            let y = ground_y - ZOMBIE_HEIGHT;
            spawn_zombie(entities, x, y)?;
        }

        self.broadcast_spawns(entities, networking, SendTarget::All)?;
        Ok(())
    }

    /// Sends an `EntitySpawnPacket` for every live zombie to the given target.
    /// Called right after seeding and again when a player joins so the new
    /// client learns about already-existing zombies.
    pub fn broadcast_spawns(&self, entities: &mut Entities, networking: &mut ServerNetworking, target: SendTarget) -> Result<()> {
        // Collect positions first so we don't call back into `entities` (and
        // look up ids) while the mutable query borrow is still live. Matching
        // the pattern in `ServerEntities::sync_entities`.
        let mut found = Vec::new();
        {
            let query = &mut entities.ecs.query::<(&PositionComponent, &ZombieComponent)>();
            for (entity, position) in query {
                found.push((entity, position.0.clone()));
            }
        }
        let mut spawns = Vec::new();
        for (entity, position) in found {
            let id = entities.get_id_from_entity(entity)?;
            spawns.push(EntitySpawnPacket {
                id,
                x: position.x(),
                y: position.y(),
                kind: EntityKind::Zombie,
            });
        }
        for spawn in spawns {
            networking.send_packet(&Packet::new(spawn)?, target.clone())?;
        }
        Ok(())
    }

    /// Advances all zombie walk logic for one server tick.
    pub fn update(&self, entities: &mut Entities, blocks: &Blocks, delta_time: f32) {
        update_zombies_ms(entities, blocks, delta_time);
    }

    /// Handles server events that matter to zombies: when a player joins we
    /// re-broadcast every live zombie's spawn so that client sees them.
    pub fn on_event(&self, event: &Event, entities: &mut Entities, networking: &mut ServerNetworking) -> Result<()> {
        if event.downcast::<crate::server::server_core::networking::NewConnectionWelcomedEvent>().is_some() {
            self.broadcast_spawns(entities, networking, SendTarget::All)?;
        }
        Ok(())
    }
}

/// Finds the y (in blocks) of the first solid block from the top of the world
/// at the given x, i.e. the surface row. Returns the world height if none found.
fn find_surface_y(blocks: &Blocks, x: f32) -> f32 {
    let height = blocks.get_size().1 as i32;
    let x = x.floor().max(0.0) as i32;
    for y in 0..height {
        if let Ok(block) = blocks.get_block_type_at(x, y) {
            if !block.ghost {
                return y as f32;
            }
        }
    }
    height as f32
}
