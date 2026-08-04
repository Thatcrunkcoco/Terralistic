# Terralistic — Session Handoff

> **Agent: read this file first.** It captures *intent and direction* only — what
> to do next and decisions-in-flight. For technical *state* (what code exists,
> what changed), rely on **git** (`git status`, `git diff`, `git log`) and the
> source. Git is the source of truth for code; this file is the source of truth
> for *what the human wants next*. Keep it lean — it's a handoff, not a changelog.

_Last updated: 2026-08-03 (gas / substance / overlay architecture session)_

---

## Next Up

- **Gas → unified substance/flux refactor** — currently mid-implementation. Completed:
  - **Step 1 (DONE):** pure rename `pressure`→`amount` across the layer/flow/server/client.
  - **Step 2 (DONE):** `GasFlow::tick` rewritten to **fixed-volume leveling + buoyancy layering**
    (3 passes: level-equalize capped by room+source, apply deduped deltas, swap unstable
    gas density ordering). `GAS_CELL_MAX_AMOUNT`=100 caps cell volume. Defaults level_rate 50 /
    buoyancy_rate 50. Buoyancy is now an unconditional swap gated by buoyancy_rate>0.
    `heavier_gas_sinks` reshaped to assert gas-identity layering. 82/82 tests pass.
  - **Remaining:** add `air`-filler removal (presentation only — remove `is_atmosphere`/
    legend air-skip in client), merge liquids as a second substance on the same layer
    (no structural change needed — just register liquid gas types), and future ONI-grade
    renderer (deferred).
  - NOTE: legacy "pressure" wording still in some test function names/comments — cosmetic.
  - Full detail + scope gauge: `session_summaries/2026-08-03-gas-substance-vision.md`.

## Decisions

- **Unify gas + liquid into one shared `SubstanceCell` layer**; keep `Gas*` naming
  ("substance" is semantic only).
- **Fixed-volume** flow (no diffusion / no expansion into vacuum); **keep buoyancy** by
  configured `density` (mass).
- **No "air" filler gas** — open space is `GasId::NONE`; breathability (if ever) is a
  property of space/room, not a gas.
- **Preserve scalability:** Activity-Scheduler wavefront (only active cells tick),
  one dense contiguous array, chunked diff-based sync, uniform-cell fast path, one
  network stream / one render pass.
- ONI × Terraria rendering direction, but the full ONI-grade smooth/high-res fluid
  renderer is **DEFERRED** to a later pass (big perf + taste risk) after the data model
  is settled. Current per-cell rendering stays.
- Overlay framework is generic (done): flowing substances live in the layer, networks
  (electrical/plumbing) live in block/tile-entity space, both feed one generic `Overlay`.
- Rust engineers hot/correct loops; Lua *declares* content. Hot paths stay out of Lua.
- Server-authoritative simulation; client reports input, server validates/corrects.
- `CONTEXT.md` holds **intent only**; git holds code state (this distinction is the
  lean philosophy).
