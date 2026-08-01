use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use serde_derive::{Deserialize, Serialize};

use crate::shared::items::{ItemId, ItemStack};
use crate::shared::scheduler::Scheduler;

/// Identifies a tile-entity *type* in the world's registry.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileEntityTypeId {
    pub(super) id: i32,
}

/// A recipe maps an input item to its output item for a processing tile entity.
#[derive(Clone)]
pub struct TileRecipe {
    pub input: ItemId,
    pub output: ItemId,
}

/// A registered tile-entity type. Lua declares *what* a block does (its name,
/// how long an operation takes, which inventory slots are input/output, and its
/// recipes); Rust executes the per-tick advancement and the recipe application.
#[derive(Clone)]
pub struct TileEntityType {
    pub name: String,
    /// Seconds for one operation (e.g. smelting a single item).
    pub tick_time: f32,
    /// Inventory slot index the entity reads input from.
    pub input_slot: usize,
    /// Inventory slot index the entity writes output to.
    pub output_slot: usize,
    /// Recipes this entity can process.
    pub recipes: Vec<TileRecipe>,
    pub(super) id: TileEntityTypeId,
}

impl TileEntityType {
    /// Creates a tile entity type. The id is a placeholder; it is assigned when
    /// the type is registered.
    #[must_use]
    pub fn new(name: String, tick_time: f32, input_slot: usize, output_slot: usize, recipes: Vec<TileRecipe>) -> Self {
        Self {
            name,
            tick_time,
            input_slot,
            output_slot,
            recipes,
            id: TileEntityTypeId { id: -1 },
        }
    }
}

/// Per-instance runtime state for a tile entity. Serialized into the block's
/// generic byte data so it persists across world save/load.
#[derive(Clone, Serialize, Deserialize)]
pub struct TileEntityState {
    /// Progress toward completing the current operation, 0..=1.
    pub progress: f32,
    /// Whether the entity is actively working. Only active entities are ticked,
    /// which keeps the update loop cheap at scale.
    pub active: bool,
}

impl TileEntityState {
    #[must_use]
    pub const fn inactive() -> Self {
        Self { progress: 0.0, active: false }
    }
}

/// Stores all tile-entity types and tracks which entities are currently active.
/// This lives inside `Blocks` and is serialized as part of the world.
#[derive(Default)]
pub struct TileEntityRegistry {
    pub(super) types: Vec<TileEntityType>,
    /// maps a block type name -> tile entity type id
    pub(super) types_by_block: HashMap<String, TileEntityTypeId>,
    /// keys (translated block index) of entities that are currently active/working
    pub(super) active: Scheduler<usize>,
}

impl TileEntityRegistry {
    /// Registers a tile entity type and binds it to the named block type.
    pub fn register(&mut self, mut ty: TileEntityType, block_name: String) -> Result<TileEntityTypeId> {
        if ty.name.is_empty() {
            bail!("Cannot register a tile entity type with an empty name");
        }
        if self.types.iter().any(|t| t.name == ty.name) {
            bail!("A tile entity type named \"{}\" already exists", ty.name);
        }
        if self.types_by_block.contains_key(&block_name) {
            bail!("Block \"{block_name}\" already has a tile entity type registered");
        }
        let id = TileEntityTypeId { id: self.types.len() as i32 };
        ty.id = id;
        self.types.push(ty);
        self.types_by_block.insert(block_name, id);
        Ok(id)
    }

    pub fn get(&self, id: TileEntityTypeId) -> Option<&TileEntityType> {
        self.types.get(id.id as usize)
    }

    /// Looks up the tile entity type bound to the given block type name, if any.
    pub fn get_by_block(&self, block_name: &str) -> Option<&TileEntityType> {
        let id = *self.types_by_block.get(block_name)?;
        self.get(id)
    }

    pub fn activate(&mut self, index: usize) {
        self.active.activate(index);
    }

    pub fn deactivate(&mut self, index: usize) {
        self.active.deactivate(index);
    }

    /// Iterates the translated indices of all currently-active entities.
    pub fn active_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.active.active()
    }
}

/// Serializes/deserializes the tile entity state into a byte blob for storage.
impl TileEntityRegistry {
    pub(super) fn pack(state: &TileEntityState) -> Result<Vec<u8>> {
        Ok(bincode::serialize(state)?)
    }

    pub(super) fn unpack(bytes: &[u8]) -> Result<TileEntityState> {
        bincode::deserialize(bytes).map_err(|e| anyhow!("failed to deserialize tile entity state: {e}"))
    }
}

/// Advances the given state by `dt` seconds toward one operation.
/// Returns true when an operation completes (progress pushed past 1).
pub(crate) fn step(state: &mut TileEntityState, dt: f32, tick_time: f32) -> bool {
    if !state.active || tick_time <= 0.0 {
        return false;
    }
    state.progress += dt / tick_time;
    if state.progress >= 1.0 {
        state.progress = 0.0;
        true
    } else {
        false
    }
}

/// Attempts to apply the entity's recipe: consumes one input item and produces
/// one output item in the output slot. Returns whether a successful exchange
/// happened and the entity can keep working.
pub(crate) fn apply_recipe(r#type: &TileEntityType, inventory: &mut Vec<Option<ItemStack>>) -> bool {
    let input_item = match inventory.get(r#type.input_slot).and_then(Option::as_ref) {
        Some(stack) => stack.item,
        None => return false,
    };

    let recipe = match r#type.recipes.iter().find(|r| r.input == input_item) {
        Some(recipe) => recipe,
        None => return false,
    };

    // Output slot must exist and be empty or already hold the same item.
    let can_produce = match inventory.get(r#type.output_slot) {
        Some(Some(existing)) => existing.item == recipe.output,
        Some(None) | None => true,
    };
    if !can_produce {
        return false;
    }

    // Produce one output.
    match inventory.get_mut(r#type.output_slot) {
        Some(Some(existing)) => existing.count += 1,
        _ => inventory[r#type.output_slot] = Some(ItemStack::new(recipe.output, 1)),
    }

    // Consume one input (remove the slot entirely if it empties).
    match inventory.get_mut(r#type.input_slot) {
        Some(Some(input)) => {
            input.count -= 1;
            if input.count <= 0 {
                inventory[r#type.input_slot] = None;
            }
        }
        _ => unreachable!("input slot was just present"),
    }

    true
}

/// Whether the entity still has a valid, placeable operation to perform. Used by
/// the tick driver to decide whether to keep an entity in the active set.
pub(crate) fn can_continue(r#type: &TileEntityType, inventory: &[Option<ItemStack>]) -> bool {
    let Some(input) = inventory.get(r#type.input_slot).and_then(Option::as_ref) else {
        return false;
    };
    if !r#type.recipes.iter().any(|r| r.input == input.item) {
        return false;
    }
    match inventory.get(r#type.output_slot) {
        None => true,
        Some(None) => true,
        Some(Some(existing)) => existing.item == r#type.recipes.iter().find(|r| r.input == input.item).map(|r| r.output).unwrap_or(existing.item),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: i32) -> ItemId {
        ItemId::from_raw(raw)
    }

    fn stack(item: ItemId, count: i32) -> Option<ItemStack> {
        Some(ItemStack::new(item, count))
    }

    fn furnace() -> TileEntityType {
        TileEntityType {
            name: "furnace".to_owned(),
            tick_time: 1.0,
            input_slot: 0,
            output_slot: 1,
            recipes: vec![
                TileRecipe { input: id(10), output: id(20) },
                TileRecipe { input: id(30), output: id(40) },
            ],
            id: TileEntityTypeId { id: 0 },
        }
    }

    #[test]
    fn step_ignores_inactive_or_zero_tick() {
        let mut state = TileEntityState::inactive();
        assert!(!step(&mut state, 1.0, 1.0));
        state.active = true;
        assert!(!step(&mut state, 1.0, 0.0));
    }

    #[test]
    fn step_completes_when_progress_passes_one() {
        let mut state = TileEntityState { progress: 0.0, active: true };
        // Two half-second steps of a 1s operation: first pushes to 0.5 (no
        // completion), second pushes past 1.0 (completes) and resets progress.
        assert!(!step(&mut state, 0.5, 1.0));
        assert!(step(&mut state, 0.5, 1.0));
        assert_eq!(state.progress, 0.0);
    }

    #[test]
    fn apply_recipe_smelts_input_to_output() {
        let ty = furnace();
        let mut inv = vec![stack(id(10), 3), None];
        assert!(apply_recipe(&ty, &mut inv));

        assert_eq!(inv[0].as_ref().unwrap().count, 2);
        assert_eq!(inv[1].as_ref().unwrap().item, id(20));
        assert_eq!(inv[1].as_ref().unwrap().count, 1);
    }

    #[test]
    fn apply_recipe_stacks_existing_output() {
        let ty = furnace();
        let mut inv = vec![stack(id(10), 1), stack(id(20), 1)];
        assert!(apply_recipe(&ty, &mut inv));
        assert_eq!(inv[1].as_ref().unwrap().count, 2);
        assert!(inv[0].is_none()); // input consumed entirely
    }

    #[test]
    fn apply_recipe_ignores_empty_input_or_unknown_recipe() {
        let ty = furnace();
        let mut empty = vec![stack(id(99), 1), None]; // no matching recipe
        assert!(!apply_recipe(&ty, &mut empty));

        let mut no_input = vec![None, None];
        assert!(!apply_recipe(&ty, &mut no_input));
    }

    #[test]
    fn registry_rejects_duplicate_and_binds_by_block() {
        let mut registry = TileEntityRegistry::default();
        assert!(registry.register(furnace(), "furnace_block".to_owned()).is_ok());
        // duplicate block binding rejected
        let mut ty2 = furnace();
        ty2.name = "furnace2".to_owned();
        assert!(registry.register(ty2, "furnace_block".to_owned()).is_err());

        assert_eq!(registry.get_by_block("furnace_block").unwrap().name, "furnace");
        assert!(registry.get_by_block("unknown").is_none());
    }

    #[test]
    fn state_pack_unpack_round_trip() {
        let state = TileEntityState { progress: 0.5, active: true };
        let bytes = TileEntityRegistry::pack(&state).unwrap();
        let unpacked = TileEntityRegistry::unpack(&bytes).unwrap();
        assert_eq!(unpacked.progress, 0.5);
        assert!(unpacked.active);
    }
}
