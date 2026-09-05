use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use serde_derive::{Deserialize, Serialize};
use snap;

use crate::libraries::events::{Event, EventManager};
use crate::shared::blocks::{Block, BlockBreakEvent, BreakingBlock, TileEntityRegistry, TileEntityState, TileEntityType, TileEntityTypeId, Tool};
use crate::shared::blocks::tile_entity::{apply_recipe, can_continue, step};
use crate::shared::items::ItemStack;
use crate::shared::world_map::WorldMap;

// width of one block in pixels
pub const BLOCK_WIDTH: f32 = 8.0;
// how many window pixels does one block pixel take
pub const RENDER_SCALE: f32 = 2.0;
// how many window pixels does one block take
pub const RENDER_BLOCK_WIDTH: f32 = BLOCK_WIDTH * RENDER_SCALE;

#[derive(Serialize, Deserialize)]
pub(super) struct BlocksData {
    pub map: WorldMap,
    pub blocks: Vec<BlockId>,
    // tells how much blocks a block in a big block is from the main block, it is mostly (0, 0) so it is stored in a hashmap
    pub block_from_main: HashMap<usize, (i32, i32)>,
    // saves the extra block data, it is mostly empty so it is stored in a hashmap
    pub block_data: HashMap<usize, Vec<u8>>,
    // saves the block inventory slots data and it is also mostly empty
    pub block_inventory_data: HashMap<usize, Vec<Option<ItemStack>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct BlockId {
    pub(super) id: i8,
}

impl BlockId {
    #[must_use]
    pub const fn undefined() -> Self {
        Self { id: -1 }
    }
}

/// A world is a 2d array of blocks.
pub struct Blocks {
    pub(super) block_data: BlocksData,
    pub(super) breaking_blocks: Vec<BreakingBlock>,
    pub(super) block_types: Vec<Block>,
    pub(super) tool_types: Vec<Tool>,
    pub(super) tile_entities: TileEntityRegistry,
    air: BlockId,
}

impl Blocks {
    #[must_use]
    pub fn new() -> Self {
        let mut result = Self {
            block_data: BlocksData {
                blocks: Vec::new(),
                block_from_main: HashMap::new(),
                block_data: HashMap::new(),
                map: WorldMap::new_empty(),
                block_inventory_data: HashMap::new(),
            },
            breaking_blocks: vec![],
            block_types: vec![],
            tool_types: vec![],
            tile_entities: TileEntityRegistry::default(),
            air: BlockId::undefined(),
        };

        let mut air = Block::new();
        air.name = "air".to_owned();
        air.ghost = true;
        air.transparent = true;
        // cannot fail: the built-in "air" block always has a valid unique name and no tool
        result.air = result.register_new_block_type(air).ok().unwrap();

        result
    }

    #[must_use]
    pub const fn get_size(&self) -> (u32, u32) {
        self.block_data.map.get_size()
    }

    #[must_use]
    pub const fn air(&self) -> BlockId {
        self.air
    }

    pub fn create(&mut self, size: (u32, u32)) {
        self.block_data.map = WorldMap::new(size);
        self.block_data.blocks = vec![self.air; (size.0 * size.1) as usize];
    }

    pub fn create_from_block_ids(&mut self, block_ids: &Vec<Vec<BlockId>>) -> Result<()> {
        let width = block_ids.len() as u32;
        let height;
        if let Some(row) = block_ids.first() {
            height = row.len() as u32;
        } else {
            bail!("Block ids must not be empty");
        }

        for row in block_ids {
            if row.len() as u32 != height {
                bail!("All rows must have the same length");
            }
        }

        self.create((width, height));
        self.block_data.blocks.clear();
        for row in block_ids {
            self.block_data.blocks.extend_from_slice(row);
        }
        Ok(())
    }

    pub fn get_block(&self, x: i32, y: i32) -> Result<BlockId> {
        // this cannot panic since the blocks vector is always the same size as the map
        #[allow(clippy::indexing_slicing)]
        Ok(self.block_data.blocks[self.block_data.map.translate_coords(x, y)?])
    }

    pub fn set_big_block(&mut self, events: &mut EventManager, x: i32, y: i32, block_id: BlockId, from_main: (i32, i32)) -> Result<()> {
        #![allow(clippy::indexing_slicing)]
        if block_id != self.get_block(x, y)? || from_main != self.get_block_from_main(x, y)? {
            let prev_block = self.get_block(x, y)?;

            self.set_block_data(x, y, vec![])?;
            // this is fine, since the blocks vector is guaranteed to be big enough
            self.block_data.blocks[self.block_data.map.translate_coords(x, y)?] = block_id;

            self.breaking_blocks.retain(|b| b.coord != (x, y));
            self.set_block_from_main(x, y, from_main)?;

            let size = self.get_block_inventory_size(x, y)? as usize;
            self.set_block_inventory_data(x, y, vec![None; size], events)?;

            let event = BlockChangeEvent { x, y, prev_block };
            events.push_event(Event::new(event));
        }
        Ok(())
    }

    pub fn set_block(&mut self, events: &mut EventManager, x: i32, y: i32, block_id: BlockId) -> Result<()> {
        self.set_big_block(events, x, y, block_id, (0, 0))
    }

    fn set_block_from_main(&mut self, x: i32, y: i32, from_main: (i32, i32)) -> Result<()> {
        let index = self.block_data.map.translate_coords(x, y)?;

        if from_main.0 == 0 && from_main.1 == 0 {
            self.block_data.block_from_main.remove(&index);
        } else {
            self.block_data.block_from_main.insert(index, from_main);
        }
        Ok(())
    }

    pub fn get_block_from_main(&self, x: i32, y: i32) -> Result<(i32, i32)> {
        Ok(self.block_data.block_from_main.get(&self.block_data.map.translate_coords(x, y)?).copied().unwrap_or((0, 0)))
    }

    pub fn set_block_data(&mut self, x: i32, y: i32, data: Vec<u8>) -> Result<()> {
        let index = self.block_data.map.translate_coords(x, y)?;
        if data.is_empty() {
            self.block_data.block_data.remove(&index);
        } else {
            self.block_data.block_data.insert(index, data);
        }
        Ok(())
    }

    pub fn get_block_inventory_size(&self, x: i32, y: i32) -> Result<i32> {
        if self.get_block_from_main(x, y)? != (0, 0) {
            return Ok(0);
        }
        Ok(self.get_block_type_at(x, y)?.inventory_slots.len() as i32)
    }

    pub fn get_block_inventory_data(&self, x: i32, y: i32) -> Result<Vec<Option<ItemStack>>> {
        Ok(self.block_data.block_inventory_data.get(&self.block_data.map.translate_coords(x, y)?).cloned().unwrap_or_else(Vec::new))
    }

    pub fn set_block_inventory_data(&mut self, x: i32, y: i32, data: Vec<Option<ItemStack>>, events: &mut EventManager) -> Result<()> {
        let index = self.block_data.map.translate_coords(x, y)?;
        let size = self.get_block_inventory_size(x, y)?;

        if size != data.len() as i32 {
            bail!("Invalid inventory size");
        }

        let prev_data = self.get_block_inventory_data(x, y)?;

        if prev_data != data {
            if data.is_empty() {
                self.block_data.block_inventory_data.remove(&index);
            } else {
                self.block_data.block_inventory_data.insert(index, data);
            }
            events.push_event(Event::new(BlockInventoryChangeEvent { x, y }));
        }
        Ok(())
    }

    pub fn get_block_data(&self, x: i32, y: i32) -> Result<Vec<u8>> {
        Ok(self.block_data.block_data.get(&self.block_data.map.translate_coords(x, y)?).cloned().unwrap_or_else(Vec::new))
    }

    pub fn serialize(&self) -> Result<Vec<u8>> {
        Ok(snap::raw::Encoder::new().compress_vec(&bincode::serialize(&self.block_data)?)?)
    }

    pub fn deserialize(&mut self, serial: &[u8]) -> Result<()> {
        self.block_data = bincode::deserialize(&snap::raw::Decoder::new().decompress_vec(serial)?)?;
        Ok(())
    }

    /// Registers a new block type, validating it first. Returns an error with a
    /// clear message (for modders) if the block has an empty/duplicate name or
    /// references a tool that hasn't been registered yet.
    pub fn register_new_block_type(&mut self, mut block_type: Block) -> Result<BlockId> {
        if block_type.name.is_empty() {
            bail!("Cannot register a block type with an empty name");
        }
        if self.block_types.iter().any(|b| b.name == block_type.name) {
            bail!("A block type named \"{}\" already exists", block_type.name);
        }
        if let Some(tool) = block_type.effective_tool {
            if self.get_tool_by_id(tool).is_none() {
                bail!(
                    "Block type \"{}\" references a tool that has not been registered yet",
                    block_type.name
                );
            }
        }

        let id = self.block_types.len() as i8;
        let result = BlockId { id };
        block_type.id = result;
        self.block_types.push(block_type);
        Ok(result)
    }

    pub fn get_block_id_by_name(&self, name: &str) -> Result<BlockId> {
        for block_type in &self.block_types {
            if block_type.name == name {
                return Ok(block_type.id);
            }
        }
        bail!("Block type not found")
    }

    #[must_use]
    pub fn get_all_block_ids(&self) -> Vec<BlockId> {
        let mut result = Vec::new();
        for block_type in &self.block_types {
            result.push(block_type.id);
        }
        result
    }

    pub fn get_block_type(&self, id: BlockId) -> Result<&Block> {
        self.block_types.get(id.id as usize).ok_or_else(|| anyhow!("Invalid block id"))
    }

    pub fn get_block_type_at(&self, x: i32, y: i32) -> Result<&Block> {
        self.get_block_type(self.get_block(x, y)?)
    }

    pub fn break_block(&mut self, events: &mut EventManager, x: i32, y: i32) -> Result<()> {
        let transformed_x = x - self.get_block_from_main(x, y)?.0;
        let transformed_y = y - self.get_block_from_main(x, y)?.1;

        let prev_block_id = self.get_block_type_at(transformed_x, transformed_y)?.id;

        let event = BlockBreakEvent {
            x: transformed_x,
            y: transformed_y,
            prev_block_id,
        };
        events.push_event(Event::new(event));

        self.set_block(events, transformed_x, transformed_y, self.air())?;

        Ok(())
    }

    // --- tile entities ---------------------------------------------------------

    pub fn register_tile_entity_type(&mut self, ty: TileEntityType, block_name: String) -> Result<TileEntityTypeId> {
        self.tile_entities.register(ty, block_name)
    }

    /// Reads the tile entity state bound to a block (at its main coordinate), if any.
    pub fn get_tile_entity_state(&self, x: i32, y: i32) -> Result<Option<TileEntityState>> {
        let (main_x, main_y) = self.get_main_block_coords(x, y)?;
        let data = self.get_block_data(main_x, main_y)?;
        if data.is_empty() {
            return Ok(None);
        }
        Ok(Some(TileEntityRegistry::unpack(&data)?))
    }

    /// Writes tile entity state for a block (at its main coordinate) and updates
    /// the activity schedule so active entities get ticked.
    pub fn set_tile_entity_state(&mut self, x: i32, y: i32, state: TileEntityState) -> Result<()> {
        let (main_x, main_y) = self.get_main_block_coords(x, y)?;
        let index = self.block_data.map.translate_coords(main_x, main_y)?;
        let packed = TileEntityRegistry::pack(&state)?;
        self.set_block_data(main_x, main_y, packed)?;
        if state.active {
            self.tile_entities.activate(index);
        } else {
            self.tile_entities.deactivate(index);
        }
        Ok(())
    }

    fn get_main_block_coords(&self, x: i32, y: i32) -> Result<(i32, i32)> {
        let from_main = self.get_block_from_main(x, y)?;
        Ok((x - from_main.0, y - from_main.1))
    }

    /// Advances all active tile entities by `dt` seconds. Only entities in the
    /// active set are processed, keeping the hot path cheap at scale. When an
    /// operation completes, the entity's recipe is applied to its inventory.
    pub fn update_tile_entities(&mut self, events: &mut EventManager, dt: f32) -> Result<()> {
        let height = self.block_data.map.get_size().1 as usize;
        let active: Vec<usize> = self.tile_entities.active_indices().collect();

        for index in active {
            let (x, y) = (index / height, index % height);
            let (main_x, main_y) = self.get_main_block_coords(x as i32, y as i32)?;

            // A printable index that always corresponds to the main block.
            let main_index = self.block_data.map.translate_coords(main_x, main_y)?;

            let data = self.get_block_data(main_x, main_y)?;
            let Some(mut state) = (if data.is_empty() {
                None
            } else {
                Some(TileEntityRegistry::unpack(&data)?)
            }) else {
                self.tile_entities.deactivate(main_index);
                continue;
            };

            let block_name = self.get_block_type_at(main_x, main_y)?.name.clone();
            let Some(ty) = self.tile_entities.get_by_block(&block_name).cloned() else {
                self.tile_entities.deactivate(main_index);
                continue;
            };

            if step(&mut state, dt, ty.tick_time) {
                // An operation completed: apply the recipe to the inventory.
                let mut inventory = self.get_block_inventory_data(main_x, main_y)?;
                if apply_recipe(&ty, &mut inventory) {
                    self.set_block_inventory_data(main_x, main_y, inventory, events)?;
                }
                // Only keep ticking if there's still useful input.
                let still_work = {
                    let inv = self.get_block_inventory_data(main_x, main_y)?;
                    can_continue(&ty, &inv)
                };
                state.active = still_work;
            }

            self.set_tile_entity_state(main_x, main_y, state)?;
        }
        Ok(())
    }

    /// Called when a block's inventory changes. If the block is a tile entity
    /// with usable input, it is activated so it gets ticked.
    pub fn activate_tile_entity_if_needed(&mut self, x: i32, y: i32) -> Result<()> {
        let (main_x, main_y) = self.get_main_block_coords(x, y)?;
        let block_name = self.get_block_type_at(main_x, main_y)?.name.clone();
        let ty = match self.tile_entities.get_by_block(&block_name).cloned() {
            Some(ty) => ty,
            None => return Ok(()),
        };

        // Read current state (or create an inactive one).
        let mut state = self.get_tile_entity_state(main_x, main_y)?.unwrap_or(TileEntityState::inactive());

        let inventory = self.get_block_inventory_data(main_x, main_y)?;
        if can_continue(&ty, &inventory) {
            state.active = true;
            self.set_tile_entity_state(main_x, main_y, state)?;
        }
        Ok(())
    }
}
pub struct BlockChangeEvent {
    pub x: i32,
    pub y: i32,
    pub prev_block: BlockId,
}

pub struct BlockRandomTickEvent {
    pub x: i32,
    pub y: i32,
}

pub struct BlockUpdateEvent {
    pub x: i32,
    pub y: i32,
}

pub struct BlockInventoryChangeEvent {
    pub x: i32,
    pub y: i32,
}

#[derive(Serialize, Deserialize)]
pub struct BlocksWelcomePacket {
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct BlockChangePacket {
    pub x: i32,
    pub y: i32,
    pub from_main_x: i32,
    pub from_main_y: i32,
    pub block: BlockId,
    pub inventory: Vec<Option<ItemStack>>,
    /// Generic per-block state blob (e.g. serialized tile entity state).
    pub state: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct BlockRightClickPacket {
    pub x: i32,
    pub y: i32,
}

// --- scratch: dump harness capture-world block ids (agent diagnostics) ---
// Run with: CAPTURE_WORLD=<save path> cargo test dump_capture_world_blocks -- --nocapture
#[test]
fn dump_capture_world_blocks() {
    let Ok(path) = std::env::var("CAPTURE_WORLD") else { println!("CAPTURE_WORLD not set; skipping"); return };
    let world_file = std::fs::read(&path).unwrap();
    let world: std::collections::HashMap<String, Vec<u8>> = bincode::deserialize(&world_file).unwrap();
    let decoded = snap::raw::Decoder::new().decompress_vec(world.get("blocks").unwrap()).unwrap();
    let data: BlocksData = bincode::deserialize(&decoded).unwrap();
    println!("map size = {:?}", data.map.get_size());
    println!("blocks len = {}", data.blocks.len());
    let zeros = data.blocks.iter().filter(|b| b.id == 0).count();
    println!("zero ids = {zeros}");
    for (x, y) in [(16u32, 5u32), (17, 10), (16, 10), (16, 11), (1, 0), (0, 0)] {
        let index = data.map.translate_coords(x as i32, y as i32).unwrap();
        println!("block ({x},{y}) = id {}", data.blocks[index].id);
    }
    // vertical slice around x=16: which rows are non-air?
    for y in 0..u32::min(24, data.map.get_size().1) {
        let index = data.map.translate_coords(16, y as i32).unwrap();
        println!("x=16 y={y} id={}", data.blocks[index].id);
    }
    // how many nonzero blocks around x in 0..30?
    for x in 0u32..30 {
        let mut counts = std::collections::HashMap::new();
        for y in 0u32..data.map.get_size().1 {
            let index = data.map.translate_coords(x as i32, y as i32).unwrap();
            *counts.entry(data.blocks[index].id).or_insert(0) += 1;
        }
        println!("x={x} id counts: {:?}", counts);
    }
}

// more scratch: row profile near spawn and around the demo box
#[test]
fn dump_capture_world_rows() {
    let Ok(path) = std::env::var("CAPTURE_WORLD") else { return };
    let world_file = std::fs::read(&path).unwrap();
    let world: std::collections::HashMap<String, Vec<u8>> = bincode::deserialize(&world_file).unwrap();
    let decoded = snap::raw::Decoder::new().decompress_vec(world.get("blocks").unwrap()).unwrap();
    let data: BlocksData = bincode::deserialize(&decoded).unwrap();
    // where do all non-air id 12 live?
    let mut hits = 0;
    for y in 0u32..data.map.get_size().1 {
        for x in 0u32..data.map.get_size().0 {
            let index = data.map.translate_coords(x as i32, y as i32).unwrap();
            let id = data.blocks[index].id;
            if id == 12 { hits += 1; println!("   id12 at ({x},{y})"); }
        }
    }
    println!("id==12 count = {hits}");
    for x in [0u32, 13, 16, 24, 32] {
        for y in 150u32..256 {
            let index = data.map.translate_coords(x as i32, y as i32).unwrap();
            let id = data.blocks[index].id;
            if id != 0 {
                print!("{id}");
            } else {
                print!(".");
            }
        }
        println!("  <- x={x}");
    }
}
