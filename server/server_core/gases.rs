use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result;

use crate::libraries::events::Event;
use crate::server::server_core::networking::{NewConnectionEvent, PacketFromClientEvent, SendTarget, ServerNetworking};
use crate::shared::blocks::Blocks;
use crate::shared::gases::{init_gases_mod_interface, ClientRequestGasDebugPacket, GasFlow, GasId, GasLayer, GasLayerUpdatePacket, GasLayerWelcomePacket, Gases, GasType};
use crate::shared::mod_manager::ModManager;
use crate::shared::packet::Packet;

/// How often (in server ticks) a gas layer snapshot is sent while a client has
/// the gas debug visualizer enabled. Kept low so the live-view feels responsive
/// without spamming large compressed payloads every tick.
const GAS_DEBUG_UPDATE_INTERVAL_TICKS: u32 = 10;

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

    /// Handles server events relevant to gases: sends the welcome layer to new
    /// connections and toggles live updates in response to client requests.
    pub fn on_event(&mut self, event: &Event, networking: &mut ServerNetworking) -> Result<()> {
        if let Some(event) = event.downcast::<NewConnectionEvent>() {
            let welcome = Packet::new(GasLayerWelcomePacket { data: self.layer.serialize()? })?;
            networking.send_packet(&welcome, SendTarget::Connection(event.conn.clone()))?;
        } else if let Some(event) = event.downcast::<PacketFromClientEvent>() {
            if let Some(packet) = event.packet.try_deserialize::<ClientRequestGasDebugPacket>() {
                if packet.enabled {
                    self.gas_debug_conns.insert(event.conn.clone());
                } else {
                    self.gas_debug_conns.remove(&event.conn);
                }
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
                let data = self.layer.serialize()?;
                let packet = Packet::new(GasLayerUpdatePacket { data })?;
                for conn in &self.gas_debug_conns {
                    networking.send_packet(&packet, SendTarget::Connection(conn.clone()))?;
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
