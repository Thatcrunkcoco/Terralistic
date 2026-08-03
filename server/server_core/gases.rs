use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result;

use crate::libraries::events::Event;
use crate::server::server_core::networking::{DisconnectEvent, NewConnectionEvent, PacketFromClientEvent, SendTarget, ServerNetworking};
use crate::shared::blocks::Blocks;
use crate::shared::gases::{init_gases_mod_interface, ClientRequestGasDebugPacket, GasCell, GasFlow, GasId, GasLayer, GasLayerUpdatePacket, GasLayerWelcomePacket, Gases, GasType};
use crate::shared::mod_manager::ModManager;
use crate::shared::packet::Packet;

/// How often (in server ticks) a gas layer snapshot is sent while a client has
/// the gas debug visualizer enabled. Kept low so the live-view feels responsive
/// without spamming large compressed payloads every tick.
const GAS_DEBUG_UPDATE_INTERVAL_TICKS: u32 = 10;

/// Maximum number of differing cells carried in a single gas-layer update chunk.
/// Each entry serializes to ~12 bytes (index + gas id + pressure), so with this
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

    /// Seeds the sealed gas demonstration room in the "test" world with the
    /// declared gas types so the debug overlay has layered content to visualize.
    /// The room interior is x in [498..503], y in [174..179] (5x5, matching the
    /// compact stone box built in `WorldGenerator::generate_test`). With only
    /// five rows we use relative bands (top heavy, middle air, bottom light) and
    /// deliberately invert them so the flow re-layers them by density.
    pub fn seed_test_gases(&mut self) {
        let (width, height) = self.layer.get_size();
        if width == 0 || height == 0 {
            return;
        }
        let id_of = |name: &str| {
            let gases = self.gases.lock().unwrap_or_else(PoisonError::into_inner);
            gases.get_gas_id_by_name(name)
        };
        let air = id_of("air").unwrap_or(GasId::NONE);
        let hydrogen = id_of("hydrogen").unwrap_or(GasId::NONE);
        let co2 = id_of("co2").unwrap_or(GasId::NONE);
        let oxygen = id_of("oxygen").unwrap_or(GasId::NONE);

        const X0: i32 = 498;
        const X1: i32 = 503;
        const Y0: i32 = 174;
        const Y1: i32 = 179;

        for y in Y0..Y1 {
            // Top band: carbon dioxide (heavy) — will settle to the bottom.
            let gas = if y < Y0 + 2 {
                co2
            } else if y < Y1 - 1 {
                // middle band: air
                air
            } else {
                // bottom band: hydrogen (light) — will float to the top
                hydrogen
            };
            if gas.is_none() {
                continue;
            }
            for x in X0..X1 {
                let _ = self.layer.set_cell(x, y, GasCell::new(gas, 100.0));
                self.flow.activate(x as u32, y as u32, width, height);
            }
        }
        // a small pocket of pure oxygen near the center for contrast
        if !oxygen.is_none() {
            for x in 500..502 {
                for y in 176..177 {
                    let _ = self.layer.set_cell(x, y, GasCell::new(oxygen, 150.0));
                    self.flow.activate(x as u32, y as u32, width, height);
                }
            }
        }

        // Second sealed demo room, x in [463..468], y in [174..179] (5x5),
        // matching the box built in `WorldGenerator::generate_test`. This room
        // is filled almost entirely with hydrogen so it renders as a distinct
        // light single-colored pocket in the debug overlay, contrasting with
        // the first room's co2/air/hydrogen mix.
        if !hydrogen.is_none() {
            for x in 463..468 {
                for y in 174..179 {
                    let _ = self.layer.set_cell(x, y, GasCell::new(hydrogen, 100.0));
                    self.flow.activate(x as u32, y as u32, width, height);
                }
            }
        }

        // Debug instrumentation: dump the seeded box cells so we can confirm the
        // layer actually holds the demo gases before any flow tick runs. Each log
        // line is `gas.raw()` (or -2 for NONE) then pressure. Room 1 is the right
        // box (x 498..503), room 2 the left box (x 463..468).
        tracing::debug!(
            "gas[seed] layer={}x{} — room1 interior:",
            width,
            height
        );
        for y in 174..179 {
            let row: Vec<String> = (498..503)
                .map(|x| match self.layer.get_cell(x, y) {
                    Ok(c) => format!("{}({:.0})", c.gas.raw(), c.pressure),
                    Err(_) => "ERR".to_owned(),
                })
                .collect();
            tracing::debug!("gas[seed]   room1 y={y}: {}", row.join(" "));
        }
        tracing::debug!("gas[seed] room2 interior:");
        for y in 174..179 {
            let row: Vec<String> = (463..468)
                .map(|x| match self.layer.get_cell(x, y) {
                    Ok(c) => format!("{}({:.0})", c.gas.raw(), c.pressure),
                    Err(_) => "ERR".to_owned(),
                })
                .collect();
            tracing::debug!("gas[seed]   room2 y={y}: {}", row.join(" "));
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
                        let key = format!("{}@{}", c.gas.raw(), c.pressure as i32);
                        *dist.entry(key).or_insert(0) += 1;
                    }
                    let mut top: Vec<_> = dist.into_iter().collect();
                    top.sort_by(|a, b| b.1.cmp(&a.1));
                    let top_s: String = top.iter().take(6).map(|(k, n)| format!("{k}:{n}")).collect::<Vec<_>>().join(" ");
                    let base = chunks.first().map(|c| {
                        format!("{}@{}", c.base.gas.raw(), c.base.pressure as i32)
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
                        Ok(c) => format!("g{}@{}", c.gas.raw(), c.pressure as i32),
                        Err(_) => "ERR".to_owned(),
                    }
                };
                tracing::debug!(
                    "gas[status] room1(500,174)={} room1(500,178)={} room2(465,174)={} room2(465,178)={}",
                    probe(500, 174),
                    probe(500, 178),
                    probe(465, 174),
                    probe(465, 178),
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
