use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result;

use crate::shared::blocks::Blocks;
use crate::shared::gases::{init_gases_mod_interface, GasFlow, GasId, GasLayer, Gases, GasType};
use crate::shared::mod_manager::ModManager;

/// Handles all gas related stuff on the server side: owns the gas registry that
/// mods populate via Lua, the per-tile gas layer, and the flow simulation that
/// runs each server tick.
///
/// The gas layer is (re)created to match the world's block layout, and each tick
/// drives the flow using the actual block data to decide which cells are open to
/// gas (ghost/air blocks) versus solid (which contain gas).
pub struct ServerGases {
    gases: Arc<Mutex<Gases>>,
    layer: GasLayer,
    flow: GasFlow,
}

impl ServerGases {
    #[must_use]
    pub fn new() -> Self {
        Self {
            gases: Arc::new(Mutex::new(Gases::new())),
            layer: GasLayer::new(),
            flow: GasFlow::new(),
        }
    }

    /// Registers the gas mod interface so `register_gas_type` / `get_gas_id_by_name`
    /// become available to Lua mods.
    pub fn init(&mut self, mods: &mut ModManager) -> Result<()> {
        init_gases_mod_interface(mods, &self.gases)
    }

    /// Rebuilds the gas layer to match the world and fills it with the default
    /// "air" gas. Call after world generation/load, when gas types are registered.
    pub fn initialize_world(&mut self, blocks: &Blocks) {
        let size = blocks.get_size();
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let air_id = self.default_gas_id();
        self.layer.create(size, air_id, 100.0);
        let (w, h) = size;
        self.flow.activate_all(w, h);
    }

    /// Resolves the default gas id: the registered gas named "air", or a built-in
    /// "air" gas registered on the fly if mods never declared one.
    fn default_gas_id(&mut self) -> GasId {
        let mut gases = self.gases.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(id) = gases.get_gas_id_by_name("air") {
            return id;
        }
        // No "air" declared by mods yet; register a sensible default.
        let id = gases
            .register_new_gas_type(GasType::new("air".to_owned(), 1.2))
            .unwrap_or_else(|_| GasId::NONE);
        if id.is_none() {
            // Nothing to work with; fall back to index 0 if any gas exists.
            GasId::from_raw(0)
        } else {
            id
        }
    }

    /// Advances the gas flow by one tick, using the current block layout to
    /// determine which cells are open to gas movement.
    pub fn update(&mut self, blocks: &Blocks) {
        let size = blocks.get_size();
        if size.0 == 0 || size.1 == 0 || self.layer.get_size() != size {
            // Layer not yet sized to the world (or world changed dimensions).
            return;
        }

        // Build an open mask from the block layout: a cell is open to gas if its
        // block is ghost (not solid), matching how "air" is defined.
        let (w, h) = size;
        let mut open = vec![false; (w * h) as usize];
        for x in 0..w as i32 {
            for y in 0..h as i32 {
                let idx = (x * h as i32 + y) as usize;
                let is_ghost = blocks
                    .get_block_type_at(x, y)
                    .map(|b| b.ghost)
                    .unwrap_or(false);
                open[idx] = is_ghost;
            }
        }

        let open_ref: &Vec<bool> = &open;
        let is_open = move |x: i32, y: i32| -> bool {
            if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                return false;
            }
            // clippy: indexing is bounds-checked above
            #[allow(clippy::indexing_slicing)]
            {
                open_ref[(x * h as i32 + y) as usize]
            }
        };

        // Lock only the `gases` field directly (not via a whole-`self` method) so
        // the density closure's borrow doesn't conflict with `&mut self.flow` /
        // `&mut self.layer` in the tick call below.
        let gases = self.gases.lock().unwrap_or_else(PoisonError::into_inner);
        let density = move |g: GasId| -> f32 {
            if g.is_none() {
                0.0
            } else {
                gases.get_gas_type(g).map(|t| t.density).unwrap_or(0.0)
            }
        };

        self.flow.tick(&mut self.layer, &is_open, &density);
    }
}

impl Default for ServerGases {
    fn default() -> Self {
        Self::new()
    }
}
