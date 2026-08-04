use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::client::game::networking::{ClientNetworking, WelcomePacketEvent};
use crate::libraries::events::Event;
use crate::shared::gases::{
    init_gases_mod_interface, ClientRequestGasDebugPacket, GasId, GasLayer, GasLayerUpdatePacket, GasLayerWelcomePacket, Gases,
};
use crate::shared::mod_manager::ModManager;
use crate::shared::packet::Packet;

/// Client-side gas registry and layer mirror.
///
/// The client does not simulate gas flow (that happens on the server), but it
/// still runs the base game mod's Lua, which declares gas types via
/// `terralistic_register_gas_type` / `terralistic_get_gas_id_by_name`. Those
/// globals must be registered here too, or Lua init would error out and the
/// client would fail to join the world.
///
/// The client also keeps a copy of the server's gas layer for the debug
/// visualizer: it receives the layer once on connection (welcome) and refreshes
/// it periodically while live updates have been requested.
pub struct ClientGases {
    gases: Arc<Mutex<Gases>>,
    layer: GasLayer,
    live_updates_requested: bool,
}

impl ClientGases {
    #[must_use]
    pub fn new() -> Self {
        Self {
            gases: Arc::new(Mutex::new(Gases::new())),
            layer: GasLayer::new(),
            live_updates_requested: false,
        }
    }

    pub fn init(&self, mods: &mut ModManager) -> Result<()> {
        init_gases_mod_interface(mods, &self.gases)
    }

    /// Handles incoming server events for the debug visualizer: the welcome
    /// layer and periodic layer updates.
    pub fn on_event(&mut self, event: &Event) -> Result<()> {
        if let Some(event) = event.downcast::<WelcomePacketEvent>() {
            if let Some(packet) = event.packet.try_deserialize::<GasLayerWelcomePacket>() {
                self.layer.deserialize(&packet.data)?;
                self.log_layer_stats("welcome");
            }
        } else if let Some(event) = event.downcast::<Packet>() {
            if let Some(packet) = event.try_deserialize::<GasLayerUpdatePacket>() {
                self.layer.apply_chunk(&packet.chunk);
                if packet.chunk.start_of_frame {
                    self.log_layer_stats("live-frame");
                }
            }
        }
        Ok(())
    }

    /// Debug helper: logs the dimensions of the mirrored gas layer plus a count
    /// of non-`air` cells, so we can see whether the test-room gases actually made
    /// it to the client. Air is identified by the gas registered under the name
    /// "air"; any cell that is empty (NONE) or a different gas counts as
    /// "non-air", which is what the overlay is meant to highlight.
    fn log_layer_stats(&self, source: &str) {
        let (w, h) = self.layer.get_size();
        let air_id = self
            .gases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_gas_id_by_name("air");
        let mut non_air = 0usize;
        let mut none = 0usize;
        let mut first_non_air: Option<(usize, i32, f32)> = None;
        for (i, c) in self.layer.cells().iter().enumerate() {
            if c.gas.is_none() {
                none += 1;
            } else if air_id != Some(c.gas) {
                non_air += 1;
                if first_non_air.is_none() {
                    first_non_air = Some((i, c.gas.raw(), c.pressure));
                }
            }
        }
        tracing::debug!(
            "gas[client/{source}] layer {}x{} total={} none={} non_air={} first_non_air={:?}",
            w,
            h,
            w.saturating_mul(h),
            none,
            non_air,
            first_non_air
        );
    }

    /// Enables or disables live gas layer updates from the server, used by the
    /// debug visualizer. When enabled, the server periodically pushes fresh
    /// snapshots; when disabled, the client keeps its last-known layer.
    pub fn request_live_updates(&mut self, enabled: bool, networking: &mut ClientNetworking) -> Result<()> {
        tracing::debug!(
            "gas_overlay: request_live_updates enabled={enabled} live_updates_requested={}",
            self.live_updates_requested
        );
        if self.live_updates_requested == enabled {
            return Ok(());
        }
        networking.send_packet(Packet::new(ClientRequestGasDebugPacket { enabled })?)?;
        self.live_updates_requested = enabled;
        tracing::debug!("gas_overlay: sent ClientRequestGasDebugPacket enabled={enabled}");
        Ok(())
    }

    /// The mirrored gas layer from the server (for the debug visualizer).
    #[must_use]
    pub fn layer(&self) -> &GasLayer {
        &self.layer
    }

    /// Returns the list of registered gas types with their overlay colors, in
    /// registration order. This drives the ONI-style legend shown next to the
    /// gas overlay so the player can map each color to a gas at a glance. The
    /// colors match [`Self::color_for_gas`] (name-based), so the legend and the
    /// cells always agree. Unknown gases fall back to a density-derived color.
    ///
    /// The atmosphere ("air") is excluded: it fills the whole world, so it is
    /// not a notable gas the player hunts for, and listing it in the key would
    /// just add noise. Only non-atmosphere gases appear.
    pub fn gas_legend(&self) -> Result<Vec<(String, crate::libraries::graphics::Color)>> {
        let gases = self.gases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let air_id = gases.get_gas_id_by_name("air");
        let mut legend = Vec::new();
        for id in gases.get_all_gas_type_ids() {
            if Some(id) == air_id {
                continue; // skip the atmosphere; it's not a highlighted gas
            }
            let gas_type = gases.get_gas_type(id)?;
            legend.push((gas_type.name.clone(), Self::gas_color(&gas_type.name, gas_type.density)));
        }
        Ok(legend)
    }

    /// Maps a gas type name to its fixed debug-overlay color. The four demo
    /// gases get explicit, easy-to-distinguish colors (air is cyan, oxygen blue,
    /// co2 green, hydrogen pink); anything unregistered falls back to a
    /// density-based ramp so it still renders sensibly.
    fn gas_color(name: &str, density: f32) -> crate::libraries::graphics::Color {
        match name {
            "air" => crate::libraries::graphics::Color::new(0, 200, 215, 255),     // cyan
            "co2" => crate::libraries::graphics::Color::new(60, 200, 60, 255),      // green
            "oxygen" => crate::libraries::graphics::Color::new(30, 100, 255, 255),  // blue
            "hydrogen" => crate::libraries::graphics::Color::new(255, 100, 180, 255), // pink
            _ => Self::density_color(density),
        }
    }

    /// Whether the given gas is the world's default breathable atmosphere
    /// (registered as "air"). The overlay treats this specially: since air fills
    /// virtually the entire world at uniform pressure, rendering it the same as
    /// notable gases would paint everything one flat color and drown out the
    /// pockets we actually want to highlight. Air is instead rendered faint and
    /// neutral so the world reads as gray and only non-air gases stand out.
    #[must_use]
    pub fn is_atmosphere(&self, gas: GasId) -> bool {
        let air_id = self
            .gases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_gas_id_by_name("air");
        air_id == Some(gas)
    }

    /// Maps a gas id to a display color for the debug overlay. Colors are fixed
    /// per gas name (air/oxygen blue, co2 green, hydrogen pink); unknown gases
    /// fall back to a density-based ramp so they still render sensibly.
    /// Empty cells read as black.
    #[must_use]
    pub fn color_for_gas(&self, gas: GasId) -> crate::libraries::graphics::Color {
        if gas.is_none() {
            return crate::libraries::graphics::Color::new(0, 0, 0, 255);
        }
        let gases = self.gases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let color = gases.get_gas_type(gas).map_or_else(
            |_| crate::libraries::graphics::Color::new(255, 0, 255, 255),
            |t| Self::gas_color(&t.name, t.density),
        );
        color
    }

    fn density_color(density: f32) -> crate::libraries::graphics::Color {
        let t = (density / 2.0).clamp(0.0, 1.0); // 0 = very light, 1 = very heavy
        let light = crate::libraries::graphics::Color::new(90, 220, 255, 255); // cyan
        let heavy = crate::libraries::graphics::Color::new(255, 90, 60, 255); // red-orange
        crate::libraries::graphics::interpolate_colors(light, heavy, t)
    }
}

impl Default for ClientGases {
    fn default() -> Self {
        Self::new()
    }
}
