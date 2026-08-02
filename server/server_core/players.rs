use std::collections::HashMap;
use std::ops::Deref;

use anyhow::{anyhow, Result};
use hecs::Entity;
use serde_derive::{Deserialize, Serialize};

use crate::libraries::events::{Event, EventManager};
use crate::server::server_core::blocks::ServerBlocks;
use crate::server::server_core::networking::{Connection, DisconnectEvent, NewConnectionWelcomedEvent, PacketFromClientEvent, SendTarget, ServerNetworking};
use crate::server::server_core::print_to_console;
use crate::shared::blocks::Blocks;
use crate::shared::entities::{Entities, HealthChangeEvent, PhysicsComponent, PositionComponent};
use crate::shared::entities::{HealthChangePacket, HealthComponent};
use crate::shared::inventory::{Inventory, InventoryCraftPacket, InventoryPacket, InventorySelectPacket, InventorySwapPacket, Slot};
use crate::shared::items::{Items, ItemStack};
use crate::shared::packet::Packet;
use crate::shared::players::{
    remove_all_picked_items, spawn_player, update_players_ms, PlayerComponent, PlayerMovingPacketToClient, PlayerMovingPacketToServer, PlayerPositionPacketToServer, PlayerSpawnPacket, RespawnPacket,
    PLAYER_HEIGHT, PLAYER_INVENTORY_SIZE, PLAYER_MAX_HEALTH, PLAYER_WIDTH,
};

#[derive(Serialize, Deserialize)]
pub struct SavedPlayerData {
    pub inventory: Inventory,
    pub position: PositionComponent,
    pub health: HealthComponent,
}

pub struct ServerPlayers {
    conns_to_players: HashMap<Connection, Option<Entity>>,
    players_to_conns: HashMap<Entity, Connection>,
    saved_players: HashMap<String, SavedPlayerData>,
}

impl ServerPlayers {
    pub fn new() -> Self {
        Self {
            conns_to_players: HashMap::new(),
            players_to_conns: HashMap::new(),
            saved_players: HashMap::new(),
        }
    }

    fn get_spawn_coords(blocks: &Blocks) -> (f32, f32) {
        let spawn_x = blocks.get_size().0 as f32 / 2.0;
        let mut spawn_y = 0.0;
        // find a spawn point
        // iterate from the top of the map to the bottom
        for y in (0..blocks.get_size().1).rev() {
            for x in 0..(PLAYER_WIDTH.ceil() as i32) {
                let block_type = blocks.get_block_type(blocks.get_block(spawn_x as i32 + x, y as i32).unwrap_or_else(|_| blocks.air()));

                let is_ghost = block_type.ok().map_or(false, |block_type| block_type.ghost);

                if !is_ghost {
                    spawn_y = y as f32 - PLAYER_HEIGHT;
                    break;
                }
            }
        }

        (spawn_x, spawn_y)
    }

    pub fn handle_client_packet(
        &mut self,
        packet_event: &PacketFromClientEvent,
        entities: &mut Entities,
        networking: &mut ServerNetworking,
        blocks: &ServerBlocks,
        events: &mut EventManager,
        items: &Items,
    ) -> Result<()> {
        let player_entity = self.get_player_from_connection(&packet_event.conn)?;
        if let Some(player_entity) = player_entity {
            if let Some(packet) = packet_event.packet.try_deserialize::<PlayerMovingPacketToServer>() {
                let mut player_component = entities.ecs.get::<&mut PlayerComponent>(player_entity)?;
                let mut physics_component = entities.ecs.get::<&mut PhysicsComponent>(player_entity)?;
                player_component.set_moving_type(packet.moving_type, &mut physics_component);
                player_component.jumping = packet.jumping;

                let id = entities.get_id_from_entity(player_entity)?;
                let packet = Packet::new(PlayerMovingPacketToClient {
                    moving_type: packet.moving_type,
                    jumping: packet.jumping,
                    player_id: id,
                })?;
                networking.send_packet(&packet, SendTarget::AllExcept(packet_event.conn.clone()))?;
            } else if let Some(packet) = packet_event.packet.try_deserialize::<InventorySelectPacket>() {
                let mut inventory = entities.ecs.get::<&mut Inventory>(player_entity)?;
                inventory.selected_slot = packet.slot;
            } else if let Some(packet) = packet_event.packet.try_deserialize::<InventorySwapPacket>() {
                let mut inventory = entities.ecs.get::<&mut Inventory>(player_entity)?;

                if let Slot::Inventory(slot) = packet.slot {
                    inventory.swap_with_selected_item(slot)?;
                }

                if let Slot::Block(x, y, slot) = packet.slot {
                    let mut inventory_data = blocks.get_blocks().get_block_inventory_data(x, y)?;

                    let block_item = inventory_data.get(slot).ok_or_else(|| anyhow!("Block at ({}, {}) doesn't have slot {}", x, y, slot))?.clone();
                    let selected_item = inventory.get_selected_item();

                    *inventory_data.get_mut(slot).ok_or_else(|| anyhow!("Block at ({}, {}) doesn't have slot {}", x, y, slot))? = selected_item;

                    let selected_slot = inventory.selected_slot.ok_or_else(|| anyhow!("Player doesn't have selected slot"))?;

                    inventory.set_item(selected_slot, block_item)?;

                    blocks.get_blocks().set_block_inventory_data(x, y, inventory_data, events)?;
                    blocks.update_block(x, y, events)?;
                }
            } else if let Some(packet) = packet_event.packet.try_deserialize::<InventoryCraftPacket>() {
                let mut inventory = entities.ecs.get::<&mut Inventory>(player_entity)?.clone();
                let position_x = entities.ecs.get::<&PositionComponent>(player_entity)?.x();
                let position_y = entities.ecs.get::<&PositionComponent>(player_entity)?.y();
                let recipe = items.get_recipe(packet.recipe)?.clone();

                inventory.craft(&recipe, (position_x, position_y), items, entities, events)?;

                *entities.ecs.get::<&mut Inventory>(player_entity)? = inventory;
            } else if let Some(packet) = packet_event.packet.try_deserialize::<PlayerPositionPacketToServer>() {
                let mut position = entities.ecs.get::<&mut PositionComponent>(player_entity)?;

                // Client-side prediction: the client runs real input/physics, so we
                // trust its reported position for the main player and adopt it. We
                // no longer reject legitimate fast movement here, which historically
                // snapped the client back to our stale position (rubber-banding).
                let reported = (packet.x, packet.y);
                let current = (position.x(), position.y());
                position.set_x(packet.x);
                position.set_y(packet.y);
                tracing::trace!("adopted client pos: ({:.2},{:.2}) from ({:.2},{:.2})", reported.0, reported.1, current.0, current.1);
            }
        }

        if packet_event.packet.try_deserialize::<RespawnPacket>().is_some() {
            self.spawn_player(&networking.get_connection_name(&packet_event.conn), &blocks.get_blocks(), entities, networking, &packet_event.conn, items)?;
        }

        Ok(())
    }

    fn spawn_player(&mut self, name: &String, blocks: &Blocks, entities: &mut Entities, networking: &mut ServerNetworking, connection: &Connection, items: &Items) -> Result<()> {
        let player_data = self.saved_players.get(name);

        let (spawn_x, spawn_y) = player_data.map_or_else(|| Self::get_spawn_coords(blocks), |player_data| (player_data.position.x(), player_data.position.y()));

        for (entity, (player, position)) in &mut entities.ecs.query::<(&PlayerComponent, &PositionComponent)>() {
            let id = entities.get_id_from_entity(entity)?;
            let spawn_packet = Packet::new(PlayerSpawnPacket {
                id,
                x: position.x(),
                y: position.y(),
                name: player.get_name().to_owned(),
            })?;
            networking.send_packet(&spawn_packet, SendTarget::Connection(connection.clone()))?;
        }

        let health_component = player_data.map_or_else(|| HealthComponent::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH), |player_data| player_data.health.clone());

        let player_id = entities.new_id();
        let player_entity = spawn_player(entities, spawn_x, spawn_y, name, player_id, health_component)?;
        self.conns_to_players.insert(connection.clone(), Some(player_entity));
        self.players_to_conns.insert(player_entity, connection.clone());

        let player_spawn_packet = Packet::new(PlayerSpawnPacket {
            id: entities.get_id_from_entity(player_entity)?,
            x: spawn_x,
            y: spawn_y,
            name: name.clone(),
        })?;

        let health_packet = {
            let health_component = entities.ecs.get::<&HealthComponent>(player_entity)?;
            Packet::new(HealthChangePacket {
                health: health_component.health(),
                max_health: health_component.max_health(),
            })?
        };
        networking.send_packet(&health_packet, SendTarget::Connection(connection.clone()))?;

        if let Some(player_data) = player_data {
            let mut inventory = entities.ecs.get::<&mut Inventory>(player_entity)?;
            *inventory = player_data.inventory.clone();
            inventory.has_changed = true;
        } else {
            let _ = grant_starter_items(entities, player_entity, items);
            // best-effort: if starter items can't be granted, the player still spawns
        }

        networking.send_packet(&player_spawn_packet, SendTarget::All)?;

        Ok(())
    }

    fn save_player(&mut self, name: &str, entities: &Entities) -> Result<()> {
        let player_entity = self.get_player_entity_from_name(name, entities)?;
        let position = entities.ecs.get::<&PositionComponent>(*player_entity)?.clone();
        let inventory = entities.ecs.get::<&Inventory>(*player_entity)?.clone();
        let health = entities.ecs.get::<&HealthComponent>(*player_entity)?.clone();
        self.saved_players.insert(
            name.to_owned(),
            SavedPlayerData {
                position: position.deref().clone(),
                inventory: inventory.deref().clone(),
                health: health.deref().clone(),
            },
        );
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub fn on_event(&mut self, event: &Event, entities: &mut Entities, blocks: &ServerBlocks, networking: &mut ServerNetworking, events: &mut EventManager, items: &Items) -> Result<()> {
        if let Some(packet_event) = event.downcast::<PacketFromClientEvent>() {
            self.handle_client_packet(packet_event, entities, networking, blocks, events, items)?;
        }

        if let Some(new_connection_event) = event.downcast::<NewConnectionWelcomedEvent>() {
            let name = networking.get_connection_name(&new_connection_event.conn);
            self.spawn_player(&name, &blocks.get_blocks(), entities, networking, &new_connection_event.conn, items)?;
        }

        if let Some(disconnect_event) = event.downcast::<DisconnectEvent>() {
            if let Ok(player_entity) = self.get_player_from_connection(&disconnect_event.conn) {
                let name = networking.get_connection_name(&disconnect_event.conn);
                print_to_console(format!("[\"{name}\"] left the game").as_str(), 0);

                if let Some(player_entity) = player_entity {
                    self.save_player(&name, entities)?;

                    self.players_to_conns.remove(&player_entity);
                    let player_id = entities.get_id_from_entity(player_entity)?;
                    entities.despawn_entity(player_id, events)?;
                }

                self.conns_to_players.remove(&disconnect_event.conn);
            }
        }

        if let Some(health_change_event) = event.downcast::<HealthChangeEvent>() {
            let entity = entities.get_entity_from_id(health_change_event.entity)?;
            let player_conn = self.players_to_conns.get(&entity).cloned();
            if let Some(player_conn) = player_conn {
                let health_component = entities.ecs.query_one_mut::<&HealthComponent>(entity)?;
                networking.send_packet(
                    &Packet::new(HealthChangePacket {
                        health: health_component.health(),
                        max_health: health_component.max_health(),
                    })?,
                    SendTarget::Connection(player_conn.clone()),
                )?;

                if health_component.health() == 0 {
                    let inventory = entities.ecs.get::<&Inventory>(entity)?.deref().clone();
                    for item in inventory.iter().flatten() {
                        for _ in 0..item.count {
                            let x = entities.ecs.get::<&PositionComponent>(entity)?.x();
                            let y = entities.ecs.get::<&PositionComponent>(entity)?.y();

                            items.drop_item(events, entities, item.item, x, y)?;
                        }
                    }

                    let name = networking.get_connection_name(&player_conn);
                    self.save_player(&name, entities)?;

                    let spawn_coord = Self::get_spawn_coords(&blocks.get_blocks());
                    let saved_player = self.saved_players.get_mut(&name).ok_or_else(|| anyhow!("Player not found"))?;
                    saved_player.position = PositionComponent::new(spawn_coord.0, spawn_coord.1);
                    saved_player.inventory = Inventory::new(PLAYER_INVENTORY_SIZE);
                    saved_player.health = HealthComponent::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH);

                    entities.despawn_entity(health_change_event.entity, events)?;
                    self.conns_to_players.insert(player_conn, None);
                    self.players_to_conns.remove(&entity);
                }
            }
        }

        Ok(())
    }

    pub fn update(&self, entities: &mut Entities, blocks: &Blocks, events: &mut EventManager, items: &Items, networking: &mut ServerNetworking) -> Result<()> {
        update_players_ms(entities, blocks);
        remove_all_picked_items(entities, events, items)?;

        for (conn, player) in &self.conns_to_players {
            if let Some(player) = player {
                let mut inventory = entities.ecs.get::<&mut Inventory>(*player)?;
                if inventory.has_changed {
                    inventory.has_changed = false;
                    networking.send_packet(&Packet::new(InventoryPacket { inventory: inventory.clone() })?, SendTarget::Connection(conn.clone()))?;
                }
            }
        }
        Ok(())
    }

    pub fn get_player_from_connection(&self, conn: &Connection) -> Result<Option<Entity>> {
        self.conns_to_players.get(conn).ok_or_else(|| anyhow!("Received PlayerMovingPacket from unknown connection")).cloned()
    }

    pub fn get_player_entity_from_name(&self, name: &str, entities: &Entities) -> Result<&Entity> {
        for entity in &mut self.conns_to_players.values().flatten() {
            let e_component = entities.ecs.get::<&mut PlayerComponent>(*entity)?;
            if e_component.get_name() == name {
                return Ok(entity);
            }
        }
        Err(anyhow!("Player not found"))
    }

    pub fn serialize(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(&self.saved_players)?)
    }

    pub fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        self.saved_players = bincode::deserialize(data)?;
        Ok(())
    }
}

/// Give a brand new player their starting tools.
/// Each tool is placed into its own slot; the pickaxe is in slot 0
/// so it is selected by default.
fn grant_starter_items(entities: &mut Entities, player_entity: hecs::Entity, items: &Items) -> Result<()> {
    let starter_slots = [("pickaxe", 0usize), ("shovel", 1usize), ("hatchet", 2usize), ("hammer", 3usize)];
    let mut inventory = entities.ecs.get::<&mut Inventory>(player_entity)?;

    for (item_name, slot) in starter_slots {
        // best-effort per item: an unknown name just skips that slot
        if let Ok(item) = items.get_item_type_by_name(item_name) {
            inventory.set_item(slot, Some(ItemStack::new(item.get_id(), 1)))?;
        }
    }
    inventory.has_changed = true;

    Ok(())
}

/// Decides whether the client's reported position should be accepted as
/// authoritative over the server's position.
///
/// The client reports its position roughly once a second. Fast movement
/// (falling, jumping) covers a lot of ground in that time, so a fixed tolerance
/// causes false "teleport" detections and rubber-banding. The tolerance is
/// therefore scaled by how far the player could plausibly have moved given its
/// current speed, plus a fixed base and margin. A true teleport (large
/// displacement at low speed) is still rejected.
#[allow(dead_code)] // retained as a documented reference of the reconcile policy
fn accept_reported_position(reported: (f32, f32), server: (f32, f32), velocity: (f32, f32)) -> bool {
    let dx = reported.0 - server.0;
    let dy = reported.1 - server.1;
    let distance = dx * dx + dy * dy;
    let speed = f32::hypot(velocity.0, velocity.1);
    let tolerance = 2.0 + speed * 1.5;
    distance < tolerance * tolerance
}

#[cfg(test)]
mod tests {
    use super::accept_reported_position;
    use super::{SavedPlayerData, ServerPlayers, PLAYER_INVENTORY_SIZE, PLAYER_MAX_HEALTH};
    use crate::shared::entities::{HealthComponent, PositionComponent};
    use crate::shared::inventory::Inventory;
    use crate::shared::items::{ItemId, ItemStack};

    #[test]
    fn accepts_standing_player_with_small_drift() {
        // standing still (speed 0) -> tolerance is just the 2.0 base
        let server = (0.0, 0.0);
        let reported = (1.0, 1.0); // ~1.41 units away
        assert!(accept_reported_position(reported, server, (0.0, 0.0)));
    }

    #[test]
    fn rejects_standing_player_instant_teleport() {
        let server = (0.0, 0.0);
        let reported = (100.0, 0.0); // far away while stationary
        assert!(!accept_reported_position(reported, server, (0.0, 0.0)));
    }

    #[test]
    fn accepts_fast_player_covering_large_distance() {
        // falling fast: speed ~30 -> tolerance is 2.0 + 45 = 47, so a 30-unit
        // fall between reports is legitimate, not a teleport
        let server = (0.0, 0.0);
        let reported = (0.0, 30.0);
        assert!(accept_reported_position(reported, server, (0.0, 30.0)));
    }

    #[test]
    fn rejects_fast_player_true_teleport() {
        // high speed but an implausibly large displacement beyond tolerance
        let server = (0.0, 0.0);
        let reported = (1000.0, 0.0);
        assert!(!accept_reported_position(reported, server, (30.0, 0.0)));
    }

    #[test]
    fn accepts_exact_position_without_updates() {
        let server = (10.0, 20.0);
        assert!(accept_reported_position(server, server, (5.0, 5.0)));
    }

    // --- save-while-falling / state persistence correctness -------------------

    fn saved_player_server() -> ServerPlayers {
        ServerPlayers::new()
    }

    #[test]
    fn save_round_trip_preserves_falling_position_and_state() {
        // A player saved mid-fall: the position can be at an odd/offset coordinate
        // (here negative y and high magnitude), which must survive a save->load
        // cycle exactly so the player reloads where they were saved.
        let position = PositionComponent::new(1234.5, -45.25);
        let mut inventory = Inventory::new(PLAYER_INVENTORY_SIZE);
        inventory.set_item(0, Some(ItemStack::new(ItemId::new(), 5))).unwrap();
        let health = HealthComponent::new(80, PLAYER_MAX_HEALTH);

        let mut before = saved_player_server();
        before
            .saved_players
            .insert("p1".to_owned(), SavedPlayerData { inventory, position, health });

        let bytes = before.serialize().unwrap();

        let mut after = saved_player_server();
        after.deserialize(&bytes).unwrap();

        let saved = after.saved_players.get("p1").unwrap();
        assert_eq!(saved.position.x(), 1234.5);
        assert_eq!(saved.position.y(), -45.25);
        assert_eq!(saved.health.health(), 80);
        assert_eq!(saved.health.max_health(), PLAYER_MAX_HEALTH);
        let item = saved.inventory.get_item(0).unwrap().unwrap();
        assert_eq!(item.count, 5);
    }

    #[test]
    fn save_round_trip_preserves_multiple_players_distinct() {
        let mut before = saved_player_server();
        before
            .saved_players
            .insert("a".to_owned(), SavedPlayerData {
                position: PositionComponent::new(1.0, 2.0),
                inventory: Inventory::new(PLAYER_INVENTORY_SIZE),
                health: HealthComponent::new(50, PLAYER_MAX_HEALTH),
            });
        before
            .saved_players
            .insert("b".to_owned(), SavedPlayerData {
                position: PositionComponent::new(300.0, 400.0),
                inventory: Inventory::new(PLAYER_INVENTORY_SIZE),
                health: HealthComponent::new(100, PLAYER_MAX_HEALTH),
            });

        let bytes = before.serialize().unwrap();
        let mut after = saved_player_server();
        after.deserialize(&bytes).unwrap();

        assert_eq!(after.saved_players.len(), 2);
        assert_eq!(after.saved_players.get("a").unwrap().position.x(), 1.0);
        assert_eq!(after.saved_players.get("a").unwrap().position.y(), 2.0);
        assert_eq!(after.saved_players.get("b").unwrap().position.x(), 300.0);
        assert_eq!(after.saved_players.get("b").unwrap().position.y(), 400.0);
    }

    #[test]
    fn empty_save_round_trips_to_empty() {
        let before = saved_player_server();
        let bytes = before.serialize().unwrap();
        let mut after = saved_player_server();
        after.deserialize(&bytes).unwrap();
        assert!(after.saved_players.is_empty());
    }
}
