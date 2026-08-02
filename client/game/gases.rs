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
            }
        } else if let Some(event) = event.downcast::<Packet>() {
            if let Some(packet) = event.try_deserialize::<GasLayerUpdatePacket>() {
                self.layer.deserialize(&packet.data)?;
            }
        }
        Ok(())
    }

    /// Enables or disables live gas layer updates from the server, used by the
    /// debug visualizer. When enabled, the server periodically pushes fresh
    /// snapshots; when disabled, the client keeps its last-known layer.
    pub fn request_live_updates(&mut self, enabled: bool, networking: &mut ClientNetworking) -> Result<()> {
        if self.live_updates_requested == enabled {
            return Ok(());
        }
        networking.send_packet(Packet::new(ClientRequestGasDebugPacket { enabled })?)?;
        self.live_updates_requested = enabled;
        Ok(())
    }

    /// The mirrored gas layer from the server (for the debug visualizer).
    #[must_use]
    pub fn layer(&self) -> &GasLayer {
        &self.layer
    }

    /// Maps a gas id to a display color for the debug overlay. Light gases (low
    /// density) read as cool cyan/blue and heavy gases as warm red/orange, so
    /// layering is intuitive at a glance. Unknown/empty gases read as magenta.
    #[must_use]
    pub fn color_for_gas(&self, gas: GasId) -> crate::libraries::graphics::Color {
        if gas.is_none() {
            return crate::libraries::graphics::Color::new(0, 0, 0, 255);
        }
        let gases = self.gases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let color = gases.get_gas_type(gas).map_or_else(
            |_| crate::libraries::graphics::Color::new(255, 0, 255, 255),
            |t| Self::density_color(t.density),
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
