use crate::client::game::gases::ClientGases;
use crate::libraries::graphics as gfx;

/// Adapts the client's mirrored gas layer to the generic `Overlay` framework.
///
/// This is the "write-a-provider" payoff of the overlay infrastructure: the
/// per-gas overlay logic (colors, legend construction, layer-size clamping)
/// lives here as pure data, while the shared `Overlay` component handles the
/// toggle, gray wash, culled/batched rendering, and legend panel.
///
/// Empty/vacuum cells (`GasId::NONE`) return `None` — nothing to draw, which
/// reads as black/empty against the gray wash. Every registered gas renders
/// vivid and fully opaque via [`ClientGases::color_for_gas`], and all
/// registered gases (including "air", which is now just a regular gas rather
/// than a world-filling atmosphere) appear in the legend.
pub struct GasOverlayProvider<'a> {
    /// Borrowed client gas state (layer + registry), not owned — the provider is
    /// short-lived and recreated/re-borrowed each frame it is used.
    pub gases: &'a ClientGases,
}

impl<'a> GasOverlayProvider<'a> {
    pub fn new(gases: &'a ClientGases) -> Self {
        Self { gases }
    }
}

impl<'a> crate::client::game::overlay::OverlayProvider for GasOverlayProvider<'a> {
    fn name(&self) -> &str {
        "Gas"
    }

    fn cell_color(&self, x: i32, y: i32) -> Option<gfx::Color> {
        let cell = self.gases.layer().get_cell(x, y).ok()?;
        if cell.gas.is_none() {
            // Vacuum: nothing to draw.
            return None;
        }
        // Every registered gas renders vivid/opaque so it's visible against the
        // gray wash. Air is no longer special — it renders like any other gas.
        let color = self.gases.color_for_gas(cell.gas);
        Some(color.set_a(235))
    }

    fn legend(&self) -> Vec<(String, gfx::Color)> {
        self.gases.gas_legend().unwrap_or_default()
    }

    fn world_size(&self) -> (u32, u32) {
        self.gases.layer().get_size()
    }
}
