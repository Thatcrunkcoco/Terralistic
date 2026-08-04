# Session Summary — 2026-08-03 (Gas / substance / overlay architecture)

## What happened

A **design-and-architecture** session (no feature code landed; see the overlay change
set below). We pressure-tested the current gas implementation and settled a directional
vision for fluids and the overlay layer, aligned to the ONI-meets-Terraria premise.

### 1. Gas simulation rethink (all decisions locked, no code yet)
The current `shared/gases/` is a **pressure-diffusion** sim: `GasCell { gas, pressure }`,
a mandated `air` filler gas that defines the ambient atmosphere, and slow gradient wash.
We decided to reshape it:

- **Fixed volume** (like liquid / Terraria-style), NOT expansion into vacuum and NOT
  pressure equalization / diffusion.
- **Keep buoyancy** — substances layer by their configured `density` (this field already
  exists as `GasType.density`; treat it as an effective mass / pseudo-mass). No new
  concept required.
- **Drop "air" as a filler.** Open space is just empty (`GasId::NONE` / amount 0) —
  no ambient-atmosphere gas. If breathability is ever added, it becomes a property of
  the *space/room*, not a gas among gases. This simplifies the overlay (no
  `is_atmosphere` special-casing) and the flow seeding.
- **Unify gas + liquid into ONE shared cell layer** (`SubstanceCell { gas, amount }`,
  keeping `Gas*` naming — "substance" is semantic, `gas`/`Gas*` is the word we keep).
  One dense `Vec`, one scheduler, one network stream, one render pass. Liquids become a
  "second substance" on the same system rather than a parallel system.
- **`Gas*` naming retained**; `density` stays as the buoyancy mass.

### 2. Scalability constraints (explicit, non-negotiable)
The refactor must NOT regress the existing scaling design. The current `GasFlow` is
built on an **Activity Scheduler wavefront** (only *active* cells iterate per tick;
perturbations propagate; stable/sealed regions cost ~0). This is the single most
important scaling property. The merge must preserve:
- Activity `Scheduler` wavefront (the #1 property).
- One dense contiguous `Vec<SubstanceCell>` (cache-friendly single pass).
- Chunked diff-based network patches (`update_chunks` / `apply_chunk`) + the
  uniform-cell fast path (static region == one diff).
- One network stream / one render pass (the structural win from the merge).

The honest perf risk going forward is the **renderer**, not the sim — ONI-style
high-res smooth fluid rendering is the likely frame-cost ceiling, so the overlay's
culling/capping (`MAX_CELLS_PER_FRAME`, viewport-clamped) matters and the data model can
be settled now without fear.

### 3. Overlay infrastructure (DONE — code landed and tests pass)
Built a **generic, data-driven overlay framework** so future overlays (gas, liquid,
electrical, plumbing, item transport) become "write a provider" instead of another
hand-rolled overlay class. Networks (electrical/plumbing) future live in block /
tile-entity space; flowing substances live in the layer — both feed the same generic
`Overlay` via a provider trait.

**Files changed:**
- Added `client/game/overlay.rs` — `OverlayProvider` trait (name, cell_color, legend,
  world_size) + reusable `Overlay` component (G-hotkey toggle, gray wash, culled+batched
  single-draw cell rendering capped at 120k cells, legend panel, public
  `iter_visible_cells`). Stores no data source — provider is passed per-call.
- Added `client/game/gas_overlay.rs` — `GasOverlayProvider` adapting `ClientGases` to
  the trait (atmosphere faint tint, vivid for notable gases, legend from the layer).
- `client/game/mod.rs` — registered `overlay` + `gas_overlay`; removed `gas_debug_overlay`.
- `client/game/core_client.rs` — wired the generic `Overlay` + `GasOverlayProvider`;
  live-update requests reconciled each frame against `overlay.is_open()` (G-key works too).
- Deleted `client/game/gas_debug_overlay.rs` (hand-rolled overlay superseded).
- Fixed a pre-existing import bug (`BaseUiElement`); removed dead `gas_color`/`density_color`
  helpers in gas_overlay.

**Verified:** `cargo check` clean (only pre-existing unrelated `UI_EVENT_SENDER`
static-mut warning remains); `cargo test` all 82/82 passing.

## Next Up
Begin the **gas → unified substance/flux data-model refactor**. Order:
1. Produce a file-by-file change plan with the exact rename map and the **flow-sim
   replacement design** (this is the one genuinely non-trivial chunk — replacing the
   pressure-diffusion solver with a fixed-volume + buoyancy relocation solver, while
   keeping the scheduler wavefront).
2. Then implement the data-model unification in `shared/gases/` first (the foundation),
   keeping the current per-cell rendering via the new provider. Liquids follow as a
   "second substance" on the same layer.

**Scope gauge (measured):** ~1,200–1,600 lines across ~10 files, one focused session.
Fan-out is clean and well-contained (server `gases.rs`, client `gases.rs`, `gas_overlay.rs`,
`mod_interface.rs`, `world_generator.rs` test boxes only) — small blast radius. 82 tests
are the safety net.

## Decisions (locked)
- Unify gas + liquid into one shared `SubstanceCell` layer; keep `Gas*` naming.
- Fixed-volume flow; no air filler; keep buoyancy by `density` (mass).
- ONI × Terraria rendering direction, but the full ONI-grade smooth/high-res fluid
  renderer is DEFERRED (big perf risk + taste-dependent — its own later pass after the
  data model is settled).
- Preserve the Activity-Scheduler wavefront + chunked diff sync + single dense array.
- Overlay framework is generic (done); networks = block/tile-entity space, substances =
- layer, both feed one generic Overlay.
