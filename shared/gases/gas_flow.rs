use crate::shared::gases::{GasCell, GasId, GasLayer};
use crate::shared::scheduler::Scheduler;

/// The per-tick flow fraction that a dial value of `100` corresponds to, for
/// `pressure_rate`.
///
/// Pressure equalization is inherently a tiny per-tick fraction of the pressure
/// difference (each cell relaxes toward its neighbors a little at a time), so a
/// raw 0..=1.0 knob would bury the useful range down near 0.01–0.08. Mapping
/// 100 to this constant spreads that range across a friendly 0-100 dial. The
/// value (0.1) sits well under the ~0.25-per-neighbor numerical stability
/// ceiling, so even `pressure_rate: 100` is stable.
const PRESSURE_RATE_MAX_FRACTION: f32 = 0.1;

/// The per-tick flow fraction that a dial value of `100` corresponds to, for
/// `buoyancy_rate` (kept smaller because buoyancy is a correction term).
const BUOYANCY_RATE_MAX_FRACTION: f32 = 0.02;

/// Parameters controlling how gas flows between cells.
///
/// Both rates are exposed as friendly **0-100 dials** (0 = disabled, 100 = the
/// practical maximum for that axis) rather than raw per-tick fractions — that
/// way the tunable range reads naturally ("dispersion at 5, buoyancy at 50")
/// instead of chasing tiny decimals. Internally each dial is scaled back to a
/// per-tick fraction via the `*_MAX_FRACTION` constants above.
#[derive(Clone, Copy, Debug)]
pub struct GasFlowParams {
    /// 0-100 dial for how quickly pressure equalizes between a cell and one
    /// neighbor. 0 disables flow entirely; higher = faster dispersion.
    pub pressure_rate: f32,
    /// 0-100 dial for the density-driven vertical bias. Positive values make a
    /// heavier gas above a lighter one sink downward (restoring stable
    /// heavy-below / light-above layering). 0 disables buoyancy.
    pub buoyancy_rate: f32,
}

impl Default for GasFlowParams {
    fn default() -> Self {
        Self {
            // Slowed to a gentle dispersion: `pressure_rate: 20` maps to a
            // per-tick fraction of ~0.02, so exposing a sealed pocket to the
            // open atmosphere disperses gradually instead of rushing out in a
            // single burst — much easier to watch the interaction.
            pressure_rate: 20.0,
            buoyancy_rate: 50.0,
        }
    }
}

impl GasFlowParams {
    /// Scales a 0-100 dial value to its per-tick flow fraction.
    fn pressure_fraction(self) -> f32 {
        (self.pressure_rate / 100.0) * PRESSURE_RATE_MAX_FRACTION
    }

    /// Scales a 0-100 dial value to its per-tick flow fraction.
    fn buoyancy_fraction(self) -> f32 {
        (self.buoyancy_rate / 100.0) * BUOYANCY_RATE_MAX_FRACTION
    }
}

/// The gas flow simulation: a pressure-relaxation step driven by an activity
/// `Scheduler`.
///
/// Scalability is the core design goal. Rather than iterating every cell in the
/// world each tick (O(grid) forever), this only iterates the cells registered as
/// *active* in the `Scheduler`. A perturbation propagates as a wavefront: cells
/// that change activate their neighbors for the next tick, and cells that reach
/// equilibrium deactivate themselves. Dormant, sealed, or stable regions therefore
/// cost essentially nothing to simulate — the same activity pattern the tile-entity
/// system uses.
///
/// This is deliberately a runtime-only structure (not serialized with the world);
/// it is rebuilt when a world loads.
pub struct GasFlow {
    /// Activity scheduler keyed by cell index (see `WorldMap::translate_coords`).
    pub active: Scheduler<usize>,
    /// Tunable behavior parameters.
    pub params: GasFlowParams,

    // Reused scratch buffers (avoid re-allocation between ticks).
    width: u32,
    height: u32,
    pressure_delta: Vec<f32>,
    inflow_gas: Vec<GasId>,
    inflow_weight: Vec<f32>,
    touched: Vec<usize>,
}

impl GasFlow {
    /// Creates a new flow simulation with default parameters and no active cells.
    #[must_use]
    pub fn new() -> Self {
        Self::with_params(GasFlowParams::default())
    }

    /// Creates a new flow simulation with the given parameters and no active cells.
    #[must_use]
    pub fn with_params(params: GasFlowParams) -> Self {
        Self {
            active: Scheduler::new(),
            params,
            width: 0,
            height: 0,
            pressure_delta: Vec::new(),
            inflow_gas: Vec::new(),
            inflow_weight: Vec::new(),
            touched: Vec::new(),
        }
    }

    /// Activates the cell at (x, y) so it participates in the next tick.
    pub fn activate(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.active.activate(translate(x, y, width, height));
    }

    /// Activates every cell in the grid. Useful for initializing a fresh world so
    /// that the whole atmosphere participates until it reaches equilibrium. Once a
    /// region stabilizes it deactivates itself.
    pub fn activate_all(&mut self, width: u32, height: u32) {
        for idx in 0..(width * height) as usize {
            self.active.activate(idx);
        }
    }

    /// Number of cells currently active.
    #[must_use]
    pub fn num_active(&self) -> usize {
        self.active.num_active()
    }

    /// Advances the gas layer by one tick of flow relaxation.
    ///
    /// - `is_open` returns `true` for a cell that gas can occupy / move through.
    ///   Gas does not flow into or out of solid (or out-of-bounds) cells, which is
    ///   what contains a sealed pocket of air.
    /// - `density` resolves a gas id to its density (supplied by the caller from
    ///   the gas registry), which drives vertical buoyancy. Empty cells (`GasId::NONE`)
    ///   are treated as density 0.
    ///
    /// The internal `#[allow(clippy::indexing_slicing)]` is safe: the scratch
    /// buffers are always sized to the grid and every index used comes from
    /// `translate` within verified bounds.
    #[allow(clippy::indexing_slicing)]
    pub fn tick(
        &mut self,
        layer: &mut GasLayer,
        is_open: &impl Fn(i32, i32) -> bool,
        density: &impl Fn(GasId) -> f32,
    ) {
        let (w, h) = layer.get_size();
        if w == 0 || h == 0 {
            return;
        }
        // Ensure scratch buffers match the grid size (re-allocated only if the
        // world dimensions actually changed, e.g. on world load).
        if self.width != w || self.height != h {
            let len = (w * h) as usize;
            self.pressure_delta = vec![0.0; len];
            self.inflow_gas = vec![GasId::NONE; len];
            self.inflow_weight = vec![0.0; len];
            self.width = w;
            self.height = h;
        } else {
            self.pressure_delta.fill(0.0);
            self.inflow_gas.fill(GasId::NONE);
            self.inflow_weight.fill(0.0);
        }
        self.touched.clear();

        // Phase 1: compute flow contributions. We read the *current* pressures
        // (a Jacobi relaxation pass) so the result is order-independent, then
        // accumulate deltas and record which gas dominates each inflow.
        let active_indices: Vec<usize> = self.active.active().collect();
        for idx in &active_indices {
            let (x, y) = untranslate(*idx, w, h);
            if !is_open(x as i32, y as i32) {
                // A solid source can't push gas around; drop it from flow.
                continue;
            }
            let cell = layer.get_cell_by_index(*idx);
            let p_c = cell.pressure;
            if p_c <= 0.0 {
                continue;
            }
            let gas_c = cell.gas;

            // 4-neighbors: (dx, dy, vertical_flag).
            // y grows downward, so dy=-1 is "up", dy=+1 is "down".
            const NEIGHBORS: [(i32, i32, f32); 4] =
                [(0, -1, 1.0), (0, 1, 1.0), (-1, 0, 0.0), (1, 0, 0.0)];

            for (dx, dy, vertical) in NEIGHBORS {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                if !is_open(nx, ny) {
                    continue;
                }
                let nidx = translate(nx as u32, ny as u32, w, h);
                let p_n = layer.pressure_by_index(nidx);

                // Pressure equalization from c -> n (dial 0-100 scaled to a
                // per-tick fraction).
                let mut flow = (p_c - p_n) * self.params.pressure_fraction();

                // Buoyancy on the vertical axis only, reinforcing stable layering:
                // a heavier gas above a lighter one sinks; it never fights an
                // already-stable heavy-below / light-above arrangement.
                if vertical > 0.0 {
                    flow += (density(gas_c) - density(layer.gas_by_index(nidx)))
                        * self.params.buoyancy_fraction();
                }

                if flow <= 0.0 {
                    continue;
                }

                self.pressure_delta[*idx] -= flow;
                self.pressure_delta[nidx] += flow;
                if flow > self.inflow_weight[nidx] {
                    self.inflow_weight[nidx] = flow;
                    self.inflow_gas[nidx] = gas_c;
                }
                self.touched.push(*idx);
                self.touched.push(nidx);
            }
        }

        // Phase 2: apply the net deltas and resolve gas types. `touched` may
        // contain each cell multiple times (a cell can flow to several
        // neighbors), and `pressure_delta[cell]` already holds the *net* change,
        // so we must deduplicate before applying. Applying an index more than
        // once would compound its delta against the already-updated pressure,
        // creating mass out of nothing.
        self.touched.sort_unstable();
        self.touched.dedup();

        for idx in &self.touched {
            let gas = layer.gas_by_index(*idx);
            let p = layer.pressure_by_index(*idx);
            let new_p = p + self.pressure_delta[*idx];

            if new_p <= 0.0 {
                // Vacuum. Go dormant; a neighbor will wake this cell if gas arrives.
                layer.set_cell_by_index(*idx, GasCell::new(GasId::NONE, 0.0));
                self.active.deactivate(*idx);
            } else {
                // Single-gas-per-cell model: adopt the gas of the dominant inflow, if any.
                let new_gas = if self.inflow_gas[*idx].is_none() {
                    gas
                } else {
                    self.inflow_gas[*idx]
                };
                layer.set_cell_by_index(*idx, GasCell::new(new_gas, new_p));
                // This cell changed; keep it active so equilibrium can continue.
                self.active.activate(*idx);
            }
        }
    }
}

impl Default for GasFlow {
    fn default() -> Self {
        Self::new()
    }
}

/// Row-major index (matching `WorldMap::translate_coords`): x * height + y.
fn translate(x: u32, y: u32, _width: u32, height: u32) -> usize {
    (x * height + y) as usize
}

fn untranslate(idx: usize, _width: u32, height: u32) -> (u32, u32) {
    let x = idx / height as usize;
    let y = idx % height as usize;
    (x as u32, y as u32)
}
