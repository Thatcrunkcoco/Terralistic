use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use hecs::Entity;
use serde_derive::{Deserialize, Serialize};

use crate::libraries::events::{Event, EventManager};
use crate::shared::blocks::Blocks;

pub const DEFAULT_GRAVITY: f32 = 80.0;
pub const FRICTION_COEFFICIENT: f32 = 0.2;
pub const AIR_RESISTANCE_COEFFICIENT: f32 = 0.005;
const DIRECTION_SIZE: f32 = 0.01;

#[must_use]
pub fn collides_with_blocks(position: &PositionComponent, physics: &PhysicsComponent, blocks: &Blocks) -> bool {
    let block_x = position.x as i32;
    let block_y = position.y as i32;

    let block_x2 = (position.x + physics.collision_width - 2.0 * DIRECTION_SIZE).ceil() as i32;
    let block_y2 = (position.y + physics.collision_height - 2.0 * DIRECTION_SIZE).ceil() as i32;

    for x in block_x..block_x2 {
        for y in block_y..block_y2 {
            let block = blocks.get_block_type_at(x, y);
            if let Ok(block) = block {
                if !block.ghost {
                    return true;
                }
            }
        }
    }

    false
}

#[must_use]
pub fn is_touching_ground(position: &PositionComponent, physics: &PhysicsComponent, blocks: &Blocks) -> bool {
    collides_with_blocks(
        &PositionComponent {
            x: position.x,
            y: position.y + DIRECTION_SIZE * 2.0,
        },
        physics,
        blocks,
    ) && physics.velocity_y.abs() <= 0.01
}

pub struct Entities {
    pub ecs: hecs::World,
    current_id: u32,
    id_to_entity: HashMap<EntityId, Entity>,
    entity_to_id: HashMap<Entity, EntityId>,
}

/// Reduce a by b, but never go below 0. if a is negative, increase it by b but never go above 0.
pub fn reduce_by(a: &mut f32, b: f32) {
    let b = b.abs();
    if *a > 0.0 {
        *a -= b;
        if *a < 0.0 {
            *a = 0.0;
        }
    } else {
        *a += b;
        if *a > 0.0 {
            *a = 0.0;
        }
    }
}

impl Entities {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ecs: hecs::World::new(),
            current_id: 0,
            id_to_entity: HashMap::new(),
            entity_to_id: HashMap::new(),
        }
    }

    pub fn update_entities_ms(&mut self, blocks: &Blocks, events: &mut EventManager) -> Result<()> {
        let mut vec = Vec::new();

        for (entity, (position, physics)) in self.ecs.query_mut::<(&mut PositionComponent, &mut PhysicsComponent)>() {
            let velocity_x_before = physics.velocity_x;
            let velocity_y_before = physics.velocity_y;

            physics.velocity_x += physics.acceleration_x / 200.0;
            physics.velocity_y += physics.acceleration_y / 200.0;

            let target_x = position.x + physics.velocity_x / 200.0;
            let target_y = position.y + physics.velocity_y / 200.0;

            let direction_x = if physics.velocity_x > 0.0 { 1.0 } else { -1.0 } * DIRECTION_SIZE;
            loop {
                if (direction_x > 0.0 && position.x > target_x + direction_x) || (direction_x < 0.0 && position.x < target_x + direction_x) {
                    position.x = target_x;
                    break;
                }

                position.x += direction_x;

                if collides_with_blocks(position, physics, blocks) {
                    position.x -= direction_x;
                    reduce_by(&mut physics.velocity_y, physics.velocity_x * FRICTION_COEFFICIENT);
                    physics.velocity_x = 0.0;
                    break;
                }
            }

            let direction_y = if physics.velocity_y > 0.0 { 1.0 } else { -1.0 } * DIRECTION_SIZE;
            loop {
                if (direction_y > 0.0 && position.y > target_y + direction_y) || (direction_y < 0.0 && position.y < target_y + direction_y) {
                    position.y = target_y;
                    break;
                }

                position.y += direction_y;

                if collides_with_blocks(position, physics, blocks) {
                    position.y -= direction_y;
                    reduce_by(&mut physics.velocity_x, physics.velocity_y * FRICTION_COEFFICIENT);
                    physics.velocity_y = 0.0;
                    break;
                }
            }

            physics.velocity_x *= 1.0 - AIR_RESISTANCE_COEFFICIENT;
            physics.velocity_y *= 1.0 - AIR_RESISTANCE_COEFFICIENT;

            let velocity_x_change = physics.velocity_x - velocity_x_before;
            let velocity_y_change = physics.velocity_y - velocity_y_before;
            let velocity_change = f32::hypot(velocity_x_change, velocity_y_change);

            vec.push((entity, velocity_change));
        }

        for (entity, velocity_change) in vec {
            let id = self.get_id_from_entity(entity)?;
            if let Ok(health_component) = self.ecs.query_one_mut::<&mut HealthComponent>(entity) {
                if velocity_change > 40.0 {
                    health_component.increase_health(-((velocity_change - 40.0) * 4.0) as i32, events, id);
                }
            }
        }

        Ok(())
    }

    pub fn assign_id(&mut self, entity: Entity, id: EntityId) -> Result<()> {
        if self.id_to_entity.contains_key(&id) {
            bail!("id already assigned");
        }
        if self.entity_to_id.contains_key(&entity) {
            bail!("entity already has assigned id");
        }

        self.id_to_entity.insert(id, entity);
        self.entity_to_id.insert(entity, id);

        Ok(())
    }

    pub fn get_entity_from_id(&self, id: EntityId) -> Result<Entity> {
        self.id_to_entity.get(&id).ok_or_else(|| anyhow!("invalid id")).copied()
    }

    pub fn get_id_from_entity(&self, entity: Entity) -> Result<EntityId> {
        self.entity_to_id.get(&entity).ok_or_else(|| anyhow!("invalid entity")).copied()
    }

    pub fn new_id(&mut self) -> EntityId {
        self.current_id += 1;
        EntityId::new(self.current_id)
    }

    pub fn despawn_entity(&mut self, id: EntityId, events: &mut EventManager) -> Result<()> {
        let entity_to_despawn = self.get_entity_from_id(id);

        if let Ok(entity) = entity_to_despawn {
            self.ecs.despawn(entity)?;
        } else {
            bail!("Could not find entity with id");
        }

        events.push_event(Event::new(EntityDespawnEvent { id }));

        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct EntityPositionVelocityPacket {
    pub id: EntityId,
    pub x: f32,
    pub y: f32,
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub force: bool,
}

#[derive(Serialize, Deserialize)]
pub struct EntityDespawnPacket {
    pub id: EntityId,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct EntityId {
    id: u32,
}

#[derive(Serialize, Deserialize)]
pub struct EntityDespawnEvent {
    pub id: EntityId,
}

impl EntityId {
    #[must_use]
    const fn new(id: u32) -> Self {
        Self { id }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PositionComponent {
    x: f32,
    y: f32,
}

impl PositionComponent {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(&self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(&self) -> f32 {
        self.y
    }

    pub fn set_x(&mut self, x: f32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: f32) {
        self.y = y;
    }
}

#[derive(Clone)]
pub struct PhysicsComponent {
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub acceleration_x: f32,
    pub acceleration_y: f32,
    collision_width: f32,
    collision_height: f32,
}

impl PhysicsComponent {
    #[must_use]
    pub const fn new(collision_width: f32, collision_height: f32) -> Self {
        Self {
            velocity_x: 0.0,
            velocity_y: 0.0,
            acceleration_x: 0.0,
            acceleration_y: DEFAULT_GRAVITY,
            collision_width,
            collision_height,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct HealthChangePacket {
    pub health: i32,
    pub max_health: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HealthComponent {
    health: i32,
    max_health: i32,
}

pub struct HealthChangeEvent {
    pub entity: EntityId,
}

impl HealthComponent {
    #[must_use]
    pub const fn new(health: i32, max_health: i32) -> Self {
        Self { health, max_health }
    }

    #[must_use]
    pub const fn health(&self) -> i32 {
        self.health
    }

    #[must_use]
    pub const fn max_health(&self) -> i32 {
        self.max_health
    }

    pub fn set_health(&mut self, health: i32, events: &mut EventManager, entity_id: EntityId) {
        let health = health.clamp(0, self.max_health);
        if self.health != health {
            self.health = health;
            events.push_event(Event::new(HealthChangeEvent { entity: entity_id }));
        }
    }

    pub fn increase_health(&mut self, health: i32, events: &mut EventManager, entity_id: EntityId) {
        self.set_health(self.health + health, events, entity_id);
    }
}

impl rlua::FromLua<'_> for EntityId {
    fn from_lua(value: rlua::Value, _lua: &rlua::Lua) -> rlua::Result<Self> {
        match value {
            rlua::Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            _ => unreachable!(),
        }
    }
}

/// make `EntityId` Lua compatible
impl rlua::UserData for EntityId {}

/// Identifies what kind of non-player entity an `EntitySpawnPacket` refers to,
/// so the client knows which renderer/lifecycle to attach.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Zombie,
}

/// Sent by the server when a non-player entity (e.g. a zombie) is spawned, so
/// the client can create the matching local entity. Without this the client
/// cannot look up the id that subsequent `EntityPositionVelocityPacket`s
/// reference.
#[derive(Serialize, Deserialize)]
pub struct EntitySpawnPacket {
    pub id: EntityId,
    pub x: f32,
    pub y: f32,
    pub kind: EntityKind,
}

/// Size of a zombie's collision box, in blocks. Matches the player so it feels
/// like a peer entity: 2 blocks wide, 3 blocks tall.
pub const ZOMBIE_WIDTH: f32 = 2.0;
pub const ZOMBIE_HEIGHT: f32 = 3.0;

/// Horizontal walk speed, in blocks per second. Server-authoritative; the client
/// only derives facing from the synced velocity.
pub const ZOMBIE_MAX_SPEED: f32 = 1.5;

/// Walking direction of a zombie. Mirrors the player's `Direction` so the client
/// can flip the sprite, but kept independent to avoid coupling to player code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ZombieDirection {
    Left,
    Right,
}

/// Marker component for a walking zombie. The client identifies zombies by
/// querying for this component; position/physics reuse the generic machinery.
pub struct ZombieComponent {
    direction: ZombieDirection,
    /// Local frame counter used to advance the walk animation. Not networked;
    /// each side advances it consistently with wall-clock time via delta_time.
    pub animation_progress: f32,
    /// Client-side interpolation target (server-authoritative snapshot). The
    /// client lerps the rendered position toward this instead of hard-snapping,
    /// so movement stays smooth between the (1 Hz) entity syncs.
    target_x: f32,
    target_y: f32,
}

impl ZombieComponent {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            direction: ZombieDirection::Right,
            animation_progress: 0.0,
            target_x: 0.0,
            target_y: 0.0,
        }
    }

    #[must_use]
    pub const fn get_direction(&self) -> ZombieDirection {
        self.direction
    }

    pub fn set_direction(&mut self, direction: ZombieDirection) {
        self.direction = direction;
    }

    #[must_use]
    pub const fn target_x(&self) -> f32 {
        self.target_x
    }

    #[must_use]
    pub const fn target_y(&self) -> f32 {
        self.target_y
    }

    pub fn set_target(&mut self, x: f32, y: f32) {
        self.target_x = x;
        self.target_y = y;
    }
}

/// Spawns a zombie entity at the given world coords (in blocks) and assigns it a
/// fresh id. Returns the spawned entity.
pub fn spawn_zombie(entities: &mut Entities, x: f32, y: f32) -> Result<hecs::Entity> {
    let mut zombie = ZombieComponent::new();
    zombie.set_target(x, y);
    let entity = entities.ecs.spawn((
        PositionComponent::new(x, y),
        PhysicsComponent::new(ZOMBIE_WIDTH, ZOMBIE_HEIGHT),
        zombie,
    ));

    let id = entities.new_id();
    entities.assign_id(entity, id)?;
    Ok(entity)
}

/// Advances every zombie for one server tick (delta_time in seconds): sets
/// horizontal velocity toward its facing and flips at the world's horizontal
/// edges so it walks back and forth. Gravity is handled by the physics step.
pub fn update_zombies_ms(entities: &mut Entities, blocks: &Blocks, delta_time: f32) {
    let world_width = blocks.get_size().0 as f32;
    let mut to_flip = Vec::new();
    for (entity, (position, zombie, physics)) in entities.ecs.query_mut::<(&PositionComponent, &mut ZombieComponent, &mut PhysicsComponent)>() {
        let dir_mult = match zombie.direction {
            ZombieDirection::Left => -1.0,
            ZombieDirection::Right => 1.0,
        };
        physics.velocity_x = dir_mult * ZOMBIE_MAX_SPEED;
        zombie.animation_progress += delta_time;

        // Out of the left or right edge of the world -> turn around next tick.
        if position.x() <= 0.0 && zombie.direction == ZombieDirection::Left {
            to_flip.push((entity, ZombieDirection::Right));
        } else if position.x() + ZOMBIE_WIDTH >= world_width && zombie.direction == ZombieDirection::Right {
            to_flip.push((entity, ZombieDirection::Left));
        }
    }

    for (entity, dir) in to_flip {
        if let Ok(zombie) = entities.ecs.query_one_mut::<&mut ZombieComponent>(entity) {
            zombie.set_direction(dir);
        }
    }
}

