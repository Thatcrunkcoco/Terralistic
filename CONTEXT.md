# Terralistic — Session Handoff

> **Agent: read this file first.** It captures *intent and direction* only — what
> to do next and decisions-in-flight. For technical *state* (what code exists,
> what changed), rely on **git** (`git status`, `git diff`, `git log`) and the
> source. Git is the source of truth for code; this file is the source of truth
> for *what the human wants next*. Keep it lean — it's a handoff, not a changelog.

_Last updated: 2026-08-03 (gas / substance / overlay architecture session)_

---

## Next Up

- **Gas → unified substance/flux refactor** (design + implement). Ordered:
  1. Produce a file-by-file change plan with the exact `Gas*`-naming rename map and the
     **flow-sim replacement design** — replace the current pressure-diffusion solver in
     `shared/gases/gas_flow.rs` with a **fixed-volume + buoyancy relocation** solver,
     preserving the Activity-Scheduler wavefront. (This is the one non-trivial chunk.)
  2. Implement the data-model unification in `shared/gases/` first (the foundation):
     collapse `GasCell { gas, pressure }` → `SubstanceCell { gas, amount }`, drop the
     `air` filler, keep `density` as buoyancy mass, and merge gas **and** liquid into ONE
     dense array / one scheduler / one network stream / one render pass. Keep `Gas*`
     naming. Liquids follow as a "second substance" on the same layer.
  - Full detail + scope gauge (~1,200–1,600 lines / ~10 files): `session_summaries/2026-08-03-gas-substance-vision.md`.
  - Overlay infra for this is DONE (generic framework + `GasOverlayProvider`) — builds clean, 82/82 tests green.

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
