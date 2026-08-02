use anyhow::{anyhow, Result};

use crate::shared::gases::GasId;
use crate::shared::world_map::WorldMap;

/// Per-tile gas state. This is the *instance* layer counterpart to `GasType`:
/// every cell in the world holds one of these, describing which gas occupies the
/// tile and how much of it is present (`pressure`).
///
/// It is deliberately tiny and densely stored — a `GasId` (a 4-byte dense index)
/// plus a pressure value — so a full world layer stays memory-cheap and
/// cache-friendly regardless of how many distinct gas types exist. Identity is
/// the compact `GasId`; density/heating behavior lives in the `GasType`.
#[derive(Clone, Copy, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct GasCell {
    /// The gas occupying this cell.
    pub gas: GasId,
    /// How much gas is present. Higher pressure = more gas. The exact scale is
    /// defined by the flow simulation (Step 3); for now it is opaque state.
    pub pressure: f32,
}

impl GasCell {
    /// A cell of the given gas with the given pressure.
    #[must_use]
    pub const fn new(gas: GasId, pressure: f32) -> Self {
        Self { gas, pressure }
    }
}

/// A dense 2D layer of gas cells, sized to the world like the block array.
///
/// Cells are stored in a contiguous `Vec` indexed by the world's
/// `translate_coords`, mirroring how `Blocks` stores its block ids. A gas cell
/// exists for every tile in the world (gas is everywhere, not sporadic), so a
/// dense array is the right scaling choice over a sparse map.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct GasLayer {
    map: WorldMap,
    cells: Vec<GasCell>,
}

impl GasLayer {
    /// Creates an empty gas layer (0x0).
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: WorldMap::new_empty(),
            cells: Vec::new(),
        }
    }

    /// Creates a gas layer of the given size, with every cell set to the given
    /// fill gas at the given pressure.
    pub fn create(&mut self, size: (u32, u32), fill_gas: GasId, fill_pressure: f32) {
        self.map = WorldMap::new(size);
        let cell = GasCell::new(fill_gas, fill_pressure);
        self.cells = vec![cell; (size.0 * size.1) as usize];
    }

    /// The size of the gas layer.
    #[must_use]
    pub const fn get_size(&self) -> (u32, u32) {
        self.map.get_size()
    }

    /// Returns the cell at (x, y).
    pub fn get_cell(&self, x: i32, y: i32) -> Result<GasCell> {
        let index = self.map.translate_coords(x, y)?;
        self.cells
            .get(index)
            .copied()
            .ok_or_else(|| anyhow!("gas cell not found"))
    }

    /// Sets the cell at (x, y). Cell writes never fail on bounds because the
    /// cells vector is kept exactly the size of the map; only out-of-bounds
    /// coordinates produce an error via `translate_coords`.
    pub fn set_cell(&mut self, x: i32, y: i32, cell: GasCell) -> Result<()> {
        let index = self.map.translate_coords(x, y)?;
        // cannot panic: cells vector is always the size of the map
        #[allow(clippy::indexing_slicing)]
        {
            self.cells[index] = cell;
        }
        Ok(())
    }

    /// Returns all cells as a slice over the dense array (row-major, indexed by
    /// `translate_coords`). Primarily for the debug visualization overlay so it
    /// can render the whole layer in one pass.
    #[must_use]
    pub fn cells(&self) -> &[GasCell] {
        &self.cells
    }

    /// Returns the cell at a raw dense index. Intended for hot-path systems
    /// (flow simulation, rendering) that already hold a translated index.
    pub fn get_cell_by_index(&self, index: usize) -> GasCell {
        // cannot panic: cells vector is always the size of the map
        #[allow(clippy::indexing_slicing)]
        {
            self.cells[index]
        }
    }

    /// Sets the cell at a raw dense index. Intended for hot-path systems that
    /// already hold a translated index.
    pub fn set_cell_by_index(&mut self, index: usize, cell: GasCell) {
        // cannot panic: cells vector is always the size of the map
        #[allow(clippy::indexing_slicing)]
        {
            self.cells[index] = cell;
        }
    }

    /// The gas id at a raw dense index.
    #[must_use]
    pub fn gas_by_index(&self, index: usize) -> GasId {
        self.get_cell_by_index(index).gas
    }

    /// The pressure at a raw dense index.
    #[must_use]
    pub fn pressure_by_index(&self, index: usize) -> f32 {
        self.get_cell_by_index(index).pressure
    }
}

impl Default for GasLayer {
    fn default() -> Self {
        Self::new()
    }
}
