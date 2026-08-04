use crate::client::game::gases::ClientGases;
use crate::libraries::graphics as gfx;

/// Adapts the client's mirrored gas layer to the generic `Overlay` framework.
///
/// This is the "write-a-provider" payoff of the overlay infrastructure: the
/// per-gas overlay logic (atmosphere special-casing, colors, legend construction,
/// layer-size clamping) lives here as pure data, while the shared `Overlay`
/// component handles the toggle, gray wash, culled/batched rendering, and legend
/// panel.
///
/// Design notes preserved from the flow this provider supersedes:
///   * "Air" is the world's ambient atmosphere filling virtually every cell at
///     uniform pressure. Painting it as vividly as notable gases would wash the
///     screen in one flat color and bury the pockets we actually care about, so
///     atmosphere cells render as a faint neutral tint (`cell_color` returns a
///     low-alpha cyan) while other gases render vivid and fully opaque.
///   * Empty/vacuum cells (`GasId::NONE`) return `None` — nothing to draw, which
///     reads as black/empty against the gray wash.
///   * The legend skips "air" so only non-atmosphere, notable gases appear.
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
        if self.gases.is_atmosphere(cell.gas) {
            // Faint ambient tint so air reads as "empty world" without drowning
            // out the notable gas pockets. Low alpha => subtle on the gray wash.
            return Some(gfx::Color::new(0, 92, 99, 90));
        }
        let color = self.gases.color_for_gas(cell.gas);
        // Notable gases render vivid/opaque so they're impossible to miss.
        Some(color.set_a(235))
    }

    fn legend(&self) -> Vec<(String, gfx::Color)> {
        self.gases.gas_legend().unwrap_or_default()
    }

    fn world_size(&self) -> (u32, u32) {
        self.gases.layer().get_size()
    }
}
