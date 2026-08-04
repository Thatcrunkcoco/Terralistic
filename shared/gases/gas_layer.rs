use anyhow::{anyhow, Result};

use crate::shared::gases::GasId;
use crate::shared::world_map::WorldMap;

/// Per-tile gas/substance state. This is the *instance* layer counterpart to
/// `GasType`: every cell in the world holds one of these, describing which
/// substance occupies the tile and how much of it is present (`amount`).
///
/// It is deliberately tiny and densely stored — a `GasId` (a 4-byte dense index)
/// plus an `amount` value — so a full world layer stays memory-cheap and
/// cache-friendly regardless of how many distinct gas types exist. Identity is
/// the compact `GasId`; density/heating behavior lives in the `GasType`.
#[derive(Clone, Copy, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct GasCell {
    /// The substance (gas or liquid) occupying this cell.
    pub gas: GasId,
    /// How much of the substance is present. Higher amount = more of it. The
    /// exact scale is defined by the (fixed-volume) flow simulation; for now it
    /// is opaque state.
    pub amount: f32,
}

impl Default for GasCell {
    fn default() -> Self {
        Self::new(GasId::NONE, 0.0)
    }
}

impl GasCell {
    /// A cell of the given gas with the given amount.
    #[must_use]
    pub const fn new(gas: GasId, amount: f32) -> Self {
        Self { gas, amount }
    }
}

/// A compact, sparse snapshot of a gas layer for transport over the network.
///
/// The full dense layer is too large for the framed-TCP wire limit once the
/// amount varies, so this sends the common `base` cell plus only the cells that differ
/// from it. A mostly-uniform atmosphere serializes to a few kilobytes.
#[derive(serde_derive::Serialize, serde_derive::Deserialize, Default, Clone)]
pub struct GasLayerPatch {
    pub width: u32,
    pub height: u32,
    /// The value filling the vast majority of cells (typically breathable air).
    pub base: GasCell,
    /// Dense indices (per `translate_coords`) that differ from `base`.
    pub indexes: Vec<u32>,
    /// The cell value for each entry in `indexes`.
    pub cells: Vec<GasCell>,
}

impl GasLayer {
    /// Builds a sparse patch of the whole layer: the most common cell becomes
    /// `base`, and only cells differing from it are listed explicitly.
    #[must_use]
    pub fn make_patch(&self) -> GasLayerPatch {
        let (w, h) = self.get_size();
        let mut counts: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
        let mut by_key: std::collections::HashMap<u64, GasCell> = std::collections::HashMap::new();
        for c in &self.cells {
            let key = ((c.gas.raw() as u64) << 32) | (c.amount as u32 as u64);
            *counts.entry(key).or_insert(0) += 1;
            by_key.entry(key).or_insert(*c);
        }
        // most common cell => base (fall back to the first cell if empty)
        let base = counts
            .iter()
            .max_by_key(|(_, n)| **n)
            .and_then(|(k, _)| by_key.get(k).copied())
            .unwrap_or_else(GasCell::default);

        let mut indexes = Vec::new();
        let mut cells = Vec::new();
        for (i, c) in self.cells.iter().enumerate() {
            if *c != base {
                indexes.push(i as u32);
                cells.push(*c);
            }
        }
        GasLayerPatch { width: w, height: h, base, indexes, cells }
    }

    /// Applies a patch to this layer, resizing/recreating it to match and filling
    /// every cell with `base` except the explicitly-listed differing cells.
    pub fn apply_patch(&mut self, patch: &GasLayerPatch) {
        if self.get_size() != (patch.width, patch.height) {
            self.map = WorldMap::new((patch.width, patch.height));
        }
        self.cells = vec![patch.base; (patch.width * patch.height) as usize];
        for (i, c) in patch.indexes.iter().zip(&patch.cells) {
            // cannot panic: index comes from the server's same-size layer
            #[allow(clippy::indexing_slicing)]
            {
                self.cells[*i as usize] = *c;
            }
        }
    }

    /// Applies a single bounded update chunk to this layer.
    ///
    /// The live-update path sends the layer's sparse patch in several bounded
    /// chunks (see [`GasLayer::update_chunks`]) so no single packet exceeds the
    /// framed-TCP wire limit. The first chunk of a frame (`start_of_frame`)
    /// resizes the layer and re-fills it with `base`; every chunk then
    /// overwrites only its explicitly-listed cells. Because chunk boundaries may
    /// fall mid-frame, this is idempotent per cell and safe to apply in any
    /// order within a frame.
    pub fn apply_chunk(&mut self, chunk: &GasLayerUpdateChunk) {
        if chunk.start_of_frame || self.get_size() != (chunk.width, chunk.height) {
            if self.get_size() != (chunk.width, chunk.height) {
                self.map = WorldMap::new((chunk.width, chunk.height));
            }
            self.cells = vec![chunk.base; (chunk.width * chunk.height) as usize];
        }
        for (i, c) in chunk.indexes.iter().zip(&chunk.cells) {
            // cannot panic: index comes from the server's same-size layer
            #[allow(clippy::indexing_slicing)]
            {
                self.cells[*i as usize] = *c;
            }
        }
    }

    /// Splits this layer's sparse patch into bounded update chunks, each sized
    /// so that no individual chunk's packet exceeds the framed-TCP wire limit.
    ///
    /// The first chunk carries `start_of_frame = true` so the client resets its
    /// base; subsequent chunks only overwrite their listed cells. This keeps the
    /// debug overlay's live view bounded even when gas/substance amounts vary across a
    /// whole open world (where a single patch would otherwise be far too large
    /// to serialize into one frame).
    #[must_use]
    pub fn update_chunks(&self, max_cells_per_chunk: usize) -> Vec<GasLayerUpdateChunk> {
        let patch = self.make_patch();
        let (w, h) = self.get_size();
        let mut chunks = Vec::new();
        let mut cells_iter = patch.cells.iter();
        for (i, chunk) in patch.indexes.chunks(max_cells_per_chunk).enumerate() {
            // `cells` is kept in lock-step with `indexes`, so we advance a single
            // cursor instead of slicing, avoiding any bounds-checked indexing.
            let start = i * max_cells_per_chunk;
            let end = (start + chunk.len()).min(patch.cells.len());
            let cell_slice = cells_iter.by_ref().take(end - start);
            chunks.push(GasLayerUpdateChunk {
                width: w,
                height: h,
                base: patch.base,
                start_of_frame: i == 0,
                indexes: chunk.to_vec(),
                cells: cell_slice.copied().collect(),
            });
        }
        if chunks.is_empty() {
            // No cells differed from base; still send one frame so the client
            // can (re)establish its base/size.
            chunks.push(GasLayerUpdateChunk {
                width: w,
                height: h,
                base: patch.base,
                start_of_frame: true,
                indexes: Vec::new(),
                cells: Vec::new(),
            });
        }
        chunks
    }
}

/// Sent from server to client once on connection, carrying a sparse snapshot of
/// the gas layer so the client has the initial atmosphere to render the overlay.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct GasLayerWelcomePacket {
    pub data: Vec<u8>,
}

/// A single bounded frame of live-update data, applied with
/// [`GasLayer::apply_chunk`].
///
/// The server sends one of these per packet so that no individual packet (and
/// therefore the framed-TCP frame carrying it) grows past the wire limit, no
/// matter how many cells differ from the base gas.
#[derive(serde_derive::Serialize, serde_derive::Deserialize, Default, Clone)]
pub struct GasLayerUpdateChunk {
    pub width: u32,
    pub height: u32,
    /// The gas cell filling every non-listed cell this frame.
    pub base: GasCell,
    /// True on the first chunk of a frame: the client resizes/re-fills its base.
    pub start_of_frame: bool,
    /// Dense indices (per `translate_coords`) to set in this chunk.
    pub indexes: Vec<u32>,
    /// The cell value for each entry in `indexes`.
    pub cells: Vec<GasCell>,
}

/// Sent periodically from server to client while the gas debug visualizer is
/// enabled, carrying one bounded chunk of a fresh gas-layer snapshot.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct GasLayerUpdatePacket {
    pub chunk: GasLayerUpdateChunk,
}


/// Sent from client to server to request (or stop) periodic gas layer updates
/// for the debug visualizer. Only ever used from the debug client.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct ClientRequestGasDebugPacket {
    pub enabled: bool,
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
    /// fill gas at the given amount.
    pub fn create(&mut self, size: (u32, u32), fill_gas: GasId, fill_amount: f32) {
        self.map = WorldMap::new(size);
        let cell = GasCell::new(fill_gas, fill_amount);
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

    /// The amount at a raw dense index.
    #[must_use]
    pub fn amount_by_index(&self, index: usize) -> f32 {
        self.get_cell_by_index(index).amount
    }

    /// Serializes the layer as a sparse, snap-compressed patch for transport.
    /// The dense layer is too large for the framed-TCP wire limit once amount
    /// varies, so we ship only the cells that differ from the common value.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let patch = self.make_patch();
        Ok(snap::raw::Encoder::new().compress_vec(&bincode::serialize(&patch)?)?)
    }

    /// Applies a layer previously produced by [`GasLayer::serialize`].
    pub fn deserialize(&mut self, serial: &[u8]) -> Result<()> {
        let patch: GasLayerPatch = bincode::deserialize(&snap::raw::Decoder::new().decompress_vec(serial)?)?;
        self.apply_patch(&patch);
        Ok(())
    }
}

impl Default for GasLayer {
    fn default() -> Self {
        Self::new()
    }
}
