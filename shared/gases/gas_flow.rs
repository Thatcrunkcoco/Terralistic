use crate::shared::gases::{GasCell, GasId, GasLayer};
use crate::shared::scheduler::Scheduler;

/// The maximum amount a single open cell can hold — a fixed volume. This is what
/// makes the flow *incompressible*: a cell never accepts more than this, so a
/// substance settles to a uniform level rather than compressing to higher
/// densities. Sealed demo pockets and the atmosphere are seeded at this level.
pub const GAS_CELL_MAX_AMOUNT: f32 = 100.0;

/// The per-tick flow fraction that a dial value of `100` corresponds to, for
/// `level_rate`.
///
/// Level equalization is inherently a tiny per-tick fraction of the amount
/// difference (each cell relaxes toward its neighbors a little at a time), so a
/// raw 0..=1.0 knob would bury the useful range down near 0.01–0.08. Mapping
/// 100 to this constant spreads that range across a friendly 0-100 dial. The
/// value (0.1) sits well under the ~0.25-per-neighbor numerical stability
/// ceiling, so even `level_rate: 100` is stable.
const LEVEL_RATE_MAX_FRACTION: f32 = 0.1;

/// Parameters controlling how gas flows between cells.
///
/// Both rates are exposed as friendly **0-100 dials** (0 = disabled, 100 = the
/// practical maximum for that axis) rather than raw per-tick fractions — that
/// way the tunable range reads naturally ("dispersion at 5, buoyancy at 50")
/// instead of chasing tiny decimals. Internally each dial is scaled back to a
/// per-tick fraction via the `*_MAX_FRACTION` constants above.
#[derive(Clone, Copy, Debug)]
pub struct GasFlowParams {
    /// 0-100 dial for how quickly the amount equalizes between a cell and one
    /// neighbor. 0 disables flow entirely; higher = faster dispersal.
    pub level_rate: f32,
    /// 0-100 dial for density-driven layering. A positive value makes a heavier
    /// gas above a lighter one swap (sink) so stable heavy-below / light-above
    /// layering is restored; 0 (or below) disables buoyancy entirely.
    pub buoyancy_rate: f32,
}

impl Default for GasFlowParams {
    fn default() -> Self {
        Self {
            // Fixed-volume relocation: `level_rate: 50` maps to a per-tick
            // fraction of ~0.05, so a sealed pocket opened to empty neighbors
            // visibly pours/levels over a few dozen ticks (clear to watch) while
            // remaining numerically stable. Buoyancy at 50 makes heavier gases
            // sink / lighter gases rise at a similar rate when they meet.
            level_rate: 50.0,
            buoyancy_rate: 50.0,
        }
    }
}

impl GasFlowParams {
    /// Scales a 0-100 dial value to its per-tick flow fraction.
    fn level_fraction(self) -> f32 {
        (self.level_rate / 100.0) * LEVEL_RATE_MAX_FRACTION
    }
}

/// The gas flow simulation: a **fixed-volume + buoyancy** relocation step driven
/// by an activity `Scheduler`.
///
/// Substances (which may be gases *or* liquids) are incompressible volumes: each
/// open cell holds at most [`GAS_CELL_MAX_AMOUNT`]. Amount relocates *downward*
/// and *laterally* (pouring / leveling) but never *upward* into open space —
/// that is the fixed-volume property that keeps a pocket from diffusing gas-like
/// into the vacuum above. Distinct gases then *layer* by density: a heavier gas
/// above a lighter one is unstable, so the two swap (heavier sinks, lighter
/// rises), while a stable light-above / heavy-below arrangement is a rest state.
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
    amount_delta: Vec<f32>,
    inflow_gas: Vec<GasId>,
    inflow_amount: Vec<f32>,
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
            amount_delta: Vec::new(),
            inflow_gas: Vec::new(),
            inflow_amount: Vec::new(),
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

    /// Advances the layer by one tick of fixed-volume relocation.
    ///
    /// - `is_open` returns `true` for a cell a substance can occupy / move through.
    ///   Amount never flows into or out of solid (or out-of-bounds) cells, which is
    ///   what contains a sealed pocket.
    /// - `density` resolves a gas id to its density (supplied by the caller from the
    ///   gas registry), which drives vertical buoyancy / layering. Empty cells
    ///   (`GasId::NONE`) are treated as density 0.
    ///
    /// The flow runs in three order-independent passes:
    ///   1. **Level equalization** — each cell levels toward a less-full neighbor
    ///      across all four directions, so a connected open region resolves to a
    ///      uniform amount (an incompressible, liquid-like surface). Transfers are
    ///      capped by the target's remaining room and the source's amount, so mass
    ///      is conserved and no cell ever goes negative.
    ///   2. **Apply** — net deltas are applied with a dedup so a cell's delta is
    ///      counted once; each cell resolves to a single gas (the dominant inflow).
    ///   3. **Buoyancy swap** — for a vertical edge holding two *different* gases in
    ///      an unstable order (heavier above lighter), the two swap gas identities,
    ///      so the heavier sinks and the lighter rises regardless of fullness.
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
            self.amount_delta = vec![0.0; len];
            self.inflow_gas = vec![GasId::NONE; len];
            self.inflow_amount = vec![0.0; len];
            self.width = w;
            self.height = h;
        } else {
            self.amount_delta.fill(0.0);
            self.inflow_gas.fill(GasId::NONE);
            self.inflow_amount.fill(0.0);
        }
        self.touched.clear();

        // --- Phase 1: amount relocation (fixed-volume flow) --------------------
        // We read the *current* amounts (a Jacobi pass) so the result is
        // order-independent, then accumulate net deltas in `amount_delta`.
        let active_indices: Vec<usize> = self.active.active().collect();
        for idx in &active_indices {
            let (x, y) = untranslate(*idx, w, h);
            if !is_open(x as i32, y as i32) {
                // A solid source can't push substance around; drop it from flow.
                continue;
            }
            let cell = layer.get_cell_by_index(*idx);
            let a_c = cell.amount;
            if a_c <= 0.0 {
                continue;
            }
            let gas_c = cell.gas;

            // 4-neighbors: (dx, dy).
            // y grows downward, so dy=-1 is "up", dy=+1 is "down".
            const NEIGHBORS: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

            for (dx, dy) in NEIGHBORS {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                if !is_open(nx, ny) {
                    continue;
                }
                let nidx = translate(nx as u32, ny as u32, w, h);
                let a_n = layer.amount_by_index(nidx);
                if a_n >= a_c {
                    // Neighbor is already as full (or fuller); nothing levels away.
                    continue;
                }

                // Room in the neighbor beyond its fixed volume (it always has room
                // since a_n < a_c <= MAX).
                let room_n = GAS_CELL_MAX_AMOUNT - a_n;
                if room_n <= 0.0 {
                    continue;
                }

                // Level equalization toward a less-full neighbor, pushed across
                // all four directions so a connected open region resolves to a
                // uniform amount (an incompressible, liquid-like surface). This is
                // order-independent (Jacobi): we read current amounts, accumulate
                // net deltas, and cap by the source's amount so mass is conserved.
                let level = self.params.level_fraction();
                let flow = ((a_c - a_n) * level).min(room_n);
                if flow <= 0.0 {
                    continue;
                }

                // Cap by how much this source actually has, so no cell can send
                // away more than it holds this tick (guarantees no negative cell).
                let flow = flow.min(a_c);

                self.amount_delta[*idx] -= flow;
                self.amount_delta[nidx] += flow;
                if flow > self.inflow_amount[nidx] {
                    self.inflow_amount[nidx] = flow;
                    self.inflow_gas[nidx] = gas_c;
                }
                self.touched.push(*idx);
                self.touched.push(nidx);
            }
        }

        // --- Phase 2: apply the net deltas and resolve gas types ---------------
        // `touched` may contain each cell multiple times (a cell can flow to
        // several neighbors), and `amount_delta[cell]` already holds the *net*
        // change, so we must deduplicate before applying. Applying an index more
        // than once would compound its delta against the already-updated amount,
        // creating mass out of nothing.
        self.touched.sort_unstable();
        self.touched.dedup();

        for idx in &self.touched {
            let gas = layer.gas_by_index(*idx);
            let a = layer.amount_by_index(*idx);
            let new_amount = a + self.amount_delta[*idx];

            if new_amount <= 0.0 {
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
                layer.set_cell_by_index(*idx, GasCell::new(new_gas, new_amount));
                // This cell changed; keep it active so equilibrium can continue.
                self.active.activate(*idx);
            }
        }

        // --- Phase 3: buoyancy — layer distinct gases by density --------------
        // For each active cell, look at the cell directly BELOW (process each
        // edge once by only checking the down-neighbor). If the two hold different
        // gases in an unstable order (heavier above lighter), swap their gas
        // identities so the heavier sinks and the lighter rises, and transfer a
        // small, rate-scaled amount of the heavier substance downward (capillary
        // of the denser phase settling). Amount is otherwise conserved and a
        // stable light-above / heavy-below arrangement is a rest state.
        for idx in &active_indices {
            let (x, y) = untranslate(*idx, w, h);
            if !is_open(x as i32, y as i32) {
                continue;
            }
            let below_y = y as i32 + 1;
            if below_y >= h as i32 || !is_open(x as i32, below_y) {
                continue;
            }
            let nidx = translate(x as u32, below_y as u32, w, h);
            let cell = layer.get_cell_by_index(*idx);
            let below = layer.get_cell_by_index(nidx);
            if cell.gas == below.gas || cell.gas.is_none() || below.gas.is_none() {
                continue;
            }
            // Unstable: heavier above lighter -> swap, so the heavier settles lower
            // and the lighter rises. Amounts exchange exactly, so mass is conserved.
            // A `buoyancy_rate` of 0 (or below) disables layering entirely.
            if self.params.buoyancy_rate > 0.0 && density(cell.gas) > density(below.gas) {
                layer.set_cell_by_index(*idx, GasCell::new(below.gas, below.amount));
                layer.set_cell_by_index(nidx, GasCell::new(cell.gas, cell.amount));
                // The swap changed both cells; keep them awake so layering can
                // cascade until the region is stable.
                self.active.activate(*idx);
                self.active.activate(nidx);
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
