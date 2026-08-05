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
    /// of non-`air` cells, so we can see whether the test-room gases actually
    /// made it to the client. "Air" is identified by the gas registered under
    /// that name; any cell that is empty (NONE) or a different gas counts as
    /// "non-air". This is purely a diagnostic counter — air is no longer special
    /// in the rendering/overlay sense, but it remains the most common demo gas,
    /// so non-air counts are a handy signal for whether test pockets arrived.
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
                    first_non_air = Some((i, c.gas.raw(), c.amount));
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

    /// Resolves everything the always-on substance renderer needs for one gas id
    /// in a single registry lock: whether it's drawable, whether it's a liquid,
    /// and its base color. The renderer caches this per distinct id per frame so
    /// it doesn't re-lock the registry for every visible cell.
    ///
    /// * **renderable** — skips vacuum (`NONE`) and plain "air": air fills most of
    ///   the world, so drawing it would just flood the screen with one tint.
    /// * **liquid** — density >= 500.0, matching the test world (gases ~1–2,
    ///   liquids 1000+), so dense substances render as solid bubbling bodies
    ///   rather than soft roiling gas.
    ///
    /// Lives here (rather than in the renderer) because color selection and the
    /// registry live under `gases`; the renderer just consumes the resolved data.
    #[must_use]
    pub fn appearance(&self, gas: GasId) -> crate::client::game::substance::GasAppearance {
        let color = self.color_for_gas(gas);
        let (renderable, liquid) = if gas.is_none() {
            (false, false)
        } else {
            let gases = self.gases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match gases.get_gas_type(gas) {
                Ok(t) => (t.name != "air", t.density >= 500.0),
                Err(_) => (false, false),
            }
        };
        crate::client::game::substance::GasAppearance { renderable, liquid, color }
    }

    /// Returns the list of registered gas types with their overlay colors, in
    /// registration order. This drives the ONI-style legend shown next to the
    /// gas overlay so the player can map each color to a gas at a glance. The
    /// colors match [`Self::color_for_gas`] (name-based), so the legend and the
    /// cells always agree. Unknown gases fall back to a density-derived color.
    ///
    /// Every registered gas appears, including "air" — which, since the switch
    /// to a fixed-volume/fixed-cell model, is just a regular gas rather than a
    /// special world-filling atmosphere.
    pub fn gas_legend(&self) -> Result<Vec<(String, crate::libraries::graphics::Color)>> {
        let gases = self.gases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut legend = Vec::new();
        for id in gases.get_all_gas_type_ids() {
            let gas_type = gases.get_gas_type(id)?;
            legend.push((gas_type.name.clone(), Self::gas_color(&gas_type.name, gas_type.density)));
        }
        Ok(legend)
    }

    /// Maps a gas type name to its fixed overlay color. The demo substances get
    /// explicit, easy-to-distinguish colors (air cyan, oxygen blue, co2 green,
    /// hydrogen pink, water deep-blue, magma orange); anything unregistered
    /// falls back to a density-based ramp so it still renders sensibly.
    ///
    /// Liquids are registered as ordinary "gas types" on the same substance
    /// layer, so they color through the exact same path as gases — no separate
    /// liquid overlay/rendering needed. Their colors are just data here.
    fn gas_color(name: &str, density: f32) -> crate::libraries::graphics::Color {
        match name {
            "air" => crate::libraries::graphics::Color::new(0, 200, 215, 255),     // cyan
            "co2" => crate::libraries::graphics::Color::new(60, 200, 60, 255),      // green
            "oxygen" => crate::libraries::graphics::Color::new(30, 100, 255, 255),  // blue
            "hydrogen" => crate::libraries::graphics::Color::new(255, 100, 180, 255), // pink
            "water" => crate::libraries::graphics::Color::new(20, 60, 220, 255),    // deep blue
            "magma" => crate::libraries::graphics::Color::new(255, 120, 30, 255),  // orange
            _ => Self::density_color(density),
        }
    }

    /// Maps a gas id to its overlay display color. Colors are fixed per gas name
    /// (air cyan, oxygen blue, co2 green, hydrogen pink); unknown gases fall
    /// back to a density-based ramp so they still render sensibly. Empty cells
    /// read as black.
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
