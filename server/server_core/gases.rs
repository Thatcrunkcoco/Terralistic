use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result;

use crate::libraries::events::Event;
use crate::server::server_core::networking::{DisconnectEvent, NewConnectionEvent, PacketFromClientEvent, SendTarget, ServerNetworking};
use crate::shared::blocks::Blocks;
use crate::shared::gases::{GAS_CELL_MAX_AMOUNT, init_gases_mod_interface, ClientRequestGasDebugPacket, GasCell, GasFlow, GasId, GasLayer, GasLayerUpdatePacket, GasLayerWelcomePacket, Gases, GasType};
use crate::shared::mod_manager::ModManager;
use crate::shared::packet::Packet;

/// How often (in server ticks) a gas layer snapshot is sent while a client has
/// the gas debug visualizer enabled. Kept low so the live-view feels responsive
/// without spamming large compressed payloads every tick.
const GAS_DEBUG_UPDATE_INTERVAL_TICKS: u32 = 10;

/// Maximum number of differing cells carried in a single gas-layer update chunk.
/// Each entry serializes to ~12 bytes (index + gas id + amount), so with this
/// cap a single chunk stays well under the ~65KB framed-TCP wire limit. The
/// snapshot is split into multiple chunks when more cells differ.
const GAS_DEBUG_CHUNK_MAX_CELLS: usize = 3000;

/// Handles all gas related stuff on the server side: owns the gas registry that
/// mods populate via Lua, the per-tile gas layer, and the flow simulation that
/// runs each server tick.
///
/// The gas layer is (re)created to match the world's block layout, and each tick
/// drives the flow using the actual block data to decide which cells are open to
/// gas (ghost/air blocks) versus solid (which contain gas).
///
/// It also serves the gas data needed by the client-side debug visualizer: it
/// sends the full layer once on connection (welcome), and periodically re-sends
/// it to any client that has explicitly requested live updates.
pub struct ServerGases {
    gases: Arc<Mutex<Gases>>,
    layer: GasLayer,
    flow: GasFlow,
    gas_debug_conns: HashSet<super::networking::Connection>,
    update_ticks: u32,
}

impl ServerGases {
    #[must_use]
    pub fn new() -> Self {
        Self {
            gases: Arc::new(Mutex::new(Gases::new())),
            layer: GasLayer::new(),
            flow: GasFlow::new(),
            gas_debug_conns: HashSet::new(),
            update_ticks: 0,
        }
    }

    /// Registers the gas mod interface so `register_gas_type` / `get_gas_id_by_name`
    /// become available to Lua mods.
    pub fn init(&mut self, mods: &mut ModManager) -> Result<()> {
        init_gases_mod_interface(mods, &self.gases)
    }

    /// Serializes the current gas layer for persistence (written into the world
    /// save file). Mirrors how blocks/walls/players are persisted so the actual
    /// gas state — including any sealed demo pockets and flow-driven layering —
    /// survives a save/reload session instead of being lost and re-seeded.
    pub fn serialize_layer(&self) -> Result<Vec<u8>> {
        self.layer.serialize()
    }

    /// Restores the gas layer from a previously-saved world file. Returns `Ok`
    /// if the blob was a valid gas layer, `Err` if it was missing/corrupt (the
    /// caller then falls back to a fresh air layer).
    ///
    /// The flow simulation's activity scheduler is a pure runtime structure and
    /// is *not* persisted with the layer, so it starts empty on every load. We
    /// therefore (re)activate the whole grid here so the restored atmosphere
    /// participates in flow again and responds to perturbations (e.g. a player
    /// breaking a block). Without this the restored cells would be frozen and
    /// gas would never move or disperse.
    pub fn deserialize_layer(&mut self, serial: &[u8]) -> Result<()> {
        self.layer.deserialize(serial)?;
        let (w, h) = self.layer.get_size();
        self.flow.activate_all(w, h);
        Ok(())
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

    /// Seeds the gas demonstration boxes in the "test" world — one sealed 5x5
    /// container per notable gas (co2, oxygen, hydrogen). The boxes sit side by
    /// side in a connected row (matching `WorldGenerator::generate_test`), so
    /// breaking the thin shared walls between neighbors lets the gases mix and
    /// re-layer by density. Air needs no container: it is the world's ambient
    /// atmosphere, so every open cell is already air and a dedicated box would
    /// render identically to (and be invisible against) the surrounding world.
    /// Interiors (left→right): co2 x[250..255], oxygen x[256..261], hydrogen
    /// x[262..267], all y in [174..179].
    pub fn seed_test_gases(&mut self) {
        let (width, height) = self.layer.get_size();
        if width == 0 || height == 0 {
            return;
        }
        let id_of = |name: &str| {
            let gases = self.gases.lock().unwrap_or_else(PoisonError::into_inner);
            gases.get_gas_id_by_name(name)
        };
        let hydrogen = id_of("hydrogen").unwrap_or(GasId::NONE);
        let co2 = id_of("co2").unwrap_or(GasId::NONE);
        let oxygen = id_of("oxygen").unwrap_or(GasId::NONE);

        // One box per non-atmosphere gas. Each 5x5 interior is filled with a
        // single gas so the overlay shows clean, comparable, distinctly-colored
        // pockets against the (air-filled) world.
        //
        // Each pocket is seeded as a FULL cell (`GAS_CELL_MAX_AMOUNT`, i.e. the
        // fixed volume of one tile) of its gas. Under fixed-volume flow there is
        // no over-pressure gradient to act on; breaking the seal instead lets the
        // pocket *pour* (downward) and *level* (laterally) into the open air and
        // separate by density. For a keep it simple:
        //   - CO2 (heaviest) pours out and sinks below the air.
        //   - oxygen (~air density) mixes / levels gently.
        //   - hydrogen (lightest) is swapped upward by buoyancy and rises out the
        //     top as a plume, since a lighter-and-below / heavier-above edge is
        //     unstable and swaps.
        let boxes: [(i32, i32, GasId); 3] = [
            (250, 255, co2),
            (256, 261, oxygen),
            (262, 267, hydrogen),
        ];
        let full_amount = GAS_CELL_MAX_AMOUNT;
        for (x0, x1, gas) in boxes {
            if gas.is_none() {
                continue;
            }
            for x in x0..x1 {
                for y in 174..179 {
                    let _ = self.layer.set_cell(x, y, GasCell::new(gas, full_amount));
                    self.flow.activate(x as u32, y as u32, width, height);
                }
            }
        }

        // Debug instrumentation: dump each seeded box so we can confirm the layer
        // holds the demo gas before any flow tick runs. Each line lists
        // `gas.raw()` (or -2 for NONE) per row.
        tracing::debug!("gas[seed] layer={}x{} — demo boxes:", width, height);
        for (label, x0, x1) in [("co2", 250, 255), ("oxygen", 256, 261), ("hydrogen", 262, 267)] {
            for y in 174..179 {
                let row: Vec<String> = (x0..x1)
                    .map(|x| match self.layer.get_cell(x, y) {
                        Ok(c) => format!("{}({:.0})", c.gas.raw(), c.amount),
                        Err(_) => "ERR".to_owned(),
                    })
                    .collect();
                tracing::debug!("gas[seed]   {label} y={y}: {}", row.join(" "));
            }
        }
    }

    /// Handles server events relevant to gases: sends the welcome layer to new
    /// connections and toggles live updates in response to client requests.
    pub fn on_event(&mut self, event: &Event, networking: &mut ServerNetworking) -> Result<()> {
        if let Some(event) = event.downcast::<NewConnectionEvent>() {
            match self.layer.serialize() {
                Ok(data) => {
                    tracing::debug!("gas: sending welcome layer ({} bytes)", data.len());
                    match Packet::new(GasLayerWelcomePacket { data }) {
                        Ok(packet) => {
                            if let Err(e) = networking.send_packet(&packet, SendTarget::Connection(event.conn.clone())) {
                                tracing::warn!("gas: welcome send to {} failed (non-fatal): {e:?}", event.conn.address);
                            }
                        }
                        Err(e) => tracing::warn!("gas: failed to build welcome packet (non-fatal): {e:?}"),
                    }
                }
                Err(e) => tracing::warn!("gas: failed to serialize welcome layer (non-fatal): {e:?}"),
            }
        } else if let Some(event) = event.downcast::<PacketFromClientEvent>() {
            if let Some(packet) = event.packet.try_deserialize::<ClientRequestGasDebugPacket>() {
                if packet.enabled {
                    self.gas_debug_conns.insert(event.conn.clone());
                    tracing::trace!("gas: client {} requested live updates", event.conn.address);
                } else {
                    self.gas_debug_conns.remove(&event.conn);
                    tracing::trace!("gas: client {} stopped live updates", event.conn.address);
                }
            }
        } else if let Some(event) = event.downcast::<DisconnectEvent>() {
            // A client that goes away while the overlay is on must not leave a
            // stale entry behind. Otherwise the next session (which reuses the
            // same loopback port) would keep receiving gas updates for a dead
            // connection, wedging the live-update bookkeeping.
            if self.gas_debug_conns.remove(&event.conn) {
                tracing::trace!("gas: disconnected client {} removed from live updates", event.conn.address);
            }
        }
        Ok(())
    }

    /// Advances the gas flow by one tick, using the current block layout to
    /// determine which cells are open to gas movement. Also (throttled) pushes
    /// fresh layer snapshots to any client with live updates enabled.
    pub fn update(&mut self, blocks: &Blocks, networking: &mut ServerNetworking) -> Result<()> {
        let size = blocks.get_size();
        if size.0 == 0 || size.1 == 0 || self.layer.get_size() != size {
            // Layer not yet sized to the world (or world changed dimensions).
            return Ok(());
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

        // Periodically feed the debug visualizer for any requesters.
        if !self.gas_debug_conns.is_empty() {
            self.update_ticks = self.update_ticks.wrapping_add(1);
            if self.update_ticks % GAS_DEBUG_UPDATE_INTERVAL_TICKS == 0 {
                // Push the live snapshot in bounded chunks so no single packet
                // (and therefore no framed-TCP frame) can exceed the wire limit,
                // even when gas pressures vary across a large open world.
                let chunks = self.layer.update_chunks(GAS_DEBUG_CHUNK_MAX_CELLS);
                // Count how many differing cells the live snapshot actually carries
                // (sum of all chunk index slices) to sanity-check the server layer.
                let diff_cells: usize = chunks.iter().map(|c| c.indexes.len()).sum();
                // Diagnostic: dump the actual gas distribution the server sees so
                // we can tell whether the layer is uniform (and in what) vs holding
                // distinct box pockets. Keyed by "gasRaw@pressure".
                {
                    let mut dist: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
                    for c in self.layer.cells() {
                        let key = format!("{}@{}", c.gas.raw(), c.amount as i32);
                        *dist.entry(key).or_insert(0) += 1;
                    }
                    let mut top: Vec<_> = dist.into_iter().collect();
                    top.sort_by(|a, b| b.1.cmp(&a.1));
                    let top_s: String = top.iter().take(6).map(|(k, n)| format!("{k}:{n}")).collect::<Vec<_>>().join(" ");
                    let base = chunks.first().map(|c| {
                        format!("{}@{}", c.base.gas.raw(), c.base.amount as i32)
                    }).unwrap_or_else(|| "none".to_owned());
                    tracing::debug!("gas[dist] n_distinct_cells={} base={base} top: {top_s}", top.len());
                }
                tracing::debug!(
                    "gas: pushing live update: {} conns, {} chunks, {} diff cells",
                    self.gas_debug_conns.len(),
                    chunks.len(),
                    diff_cells
                );
                // Periodic confirmation that the two demo rooms still hold gas
                // (sparse dump of one representative column each, once per update).
                let probe = |x: i32, y: i32| -> String {
                    match self.layer.get_cell(x, y) {
                        Ok(c) => format!("g{}@{}", c.gas.raw(), c.amount as i32),
                        Err(_) => "ERR".to_owned(),
                    }
                };
                tracing::debug!(
                    "gas[status] co2(252,174)={} oxygen(258,174)={} hydrogen(264,174)={}",
                    probe(252, 174),
                    probe(258, 174),
                    probe(264, 174),
                );
                for chunk in chunks {
                    let packet = Packet::new(GasLayerUpdatePacket { chunk })?;
                    for conn in &self.gas_debug_conns {
                        networking.send_packet(&packet, SendTarget::Connection(conn.clone()))?;
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for ServerGases {
    fn default() -> Self {
        Self::new()
    }
}
