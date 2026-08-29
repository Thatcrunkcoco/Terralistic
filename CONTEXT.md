# Terralistic — Session Handoff

> **Agent: read this file first.** It captures *intent and direction* only — what
> to do next and decisions-in-flight. For technical *state* (what code exists,
> what changed), rely on **git** (`git status`, `git diff`, `git log`) and the
> source. Git is the source of truth for code; this file is the source of truth
> for *what the human wants next*. Keep it lean — it's a handoff, not a changelog.

_Last updated: 2026-08-28 (zombie entity basics + smooth walk animation/motion)_

---

## Next Up

- **Entities — zombies.** First non-player entity is in: a procedural, code-drawn
  zombie that walks back and forth across the flat test world (name `"test"`, seed `123`),
  seeded just right of the demo gas/liquid structures. Rendering is generated in code
  (no PNGs): a 16-frame baked walk strip with sub-pixel limb sway + body bob; motion is
  smoothed three ways — no `.round()` in the draw position, 16-frame walk cycle paced at
  1s/cycle, and **client-side position interpolation** (the server syncs at 1 Hz, so the
  client lerps toward the authoritative snapshot instead of hard-snapping, which was the
  "pops block by block" root cause). Full detail:
  `session_summaries/2026-08-28-entity-zombie-smooth-walk.md`.
  - **Awaiting final user approval** — bodies now glide/pacing is smooth; next open ends
    below.
- **Remaining (not yet scoped):**
  - **Zombie follow-ons:** AI (chase the player), collision with/around blocks while
    walking, facing derivation is server-side velocity only (fine), and reconciling the
    animation cadence (`WALK_CYCLE_SECONDS`) with actual walk speed so steps visually
    cover the right ground distance per cycle.
  - **Substance rendering — the previously-deferred ONI-grade cartoon renderer is DONE.**
  Gases and liquids now render as smooth, gooey, cartoon fluids via a **field-texture**
  approach (bake the visible region into an RGBA GPU texture at 1 texel/tile, draw ONE
  region quad, let the GPU bilinear + a 3×3 in-shader gaussian melt tile steps into a
  continuous body; surface-wave displacement + continuous noise for roil/bubbles/motes).
  Liquids = "gas mechanics toned up" (richer color, denser bubbles); the gas look the
  user approved stays unchanged. Fluid now **stops at solid-block edges** (masked out of
  walls CPU-side + shader wave freezes via a `block_mask` texture). Full detail:
  `session_summaries/2026-08-04-substance-cartoon-renderer.md`.
  - **Awaiting final user approval** after rebuild/relaunch (especially the wall
    containment + the 3×3 blur kernel from the perf pass).
- **Remaining (not yet scoped):**
  - **Flow/containment mechanics not built** — the fluid sim ignores walls; the current
    containment is purely visual masking at render time. When real wall-collision flow is
    added, revisit so visual mask + sim cooperate.
  - **Fidelity roadmap:** [3] finer sim data (amount-as-pressure sub-tile), [2]
    heightfield/vertex surface waves, [4] post layers (glow/blur + specular), and a
    `SUBTILE` field-resolution bump (more texels/tile) if liquid needs crisper silhouettes.
  - **Per-substance `viscosity`** field on `GasType` so liquids level/flow slower than
    gases (clean generic mechanism).
  - **New overlays** (plumbing, electrical, item transport) as additional
    `OverlayProvider`s.
  - Clean up legacy "pressure" wording in a few test function names/comments (cosmetic).

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
- ONI × Terraria rendering direction, achieved procedurally (no/minimal manual art):
  **field-texture cartoon renderer** for gases + liquids. Field stays at **1 texel/tile**
  (cheap upload); smoothing lives in the GPU shader kernel + bilinear — **not** CPU
  pre-blur (a v2.6 CPU-pre-blur experiment was reverted; the user preferred the GPU-blur
  gas look). Liquids = gas mechanics "toned up" (stronger color, denser bubbles).
- **Animated effects must use continuous noise, never per-cell hashes** — the recurring
  "chunking" / "square blocks aliasing" bug root cause. `hash21(floor())` quantizes to a
  coarse grid; `wobble()`/`sin()` on smooth coords stays smooth.
- **Fluids visually respect solid (non-ghost) blocks**: masked out of walls + wave
  freezes at the edge via a `block_mask` texture (render-time only, until flow mechanics
  exist).
- Overlay framework is generic (done): flowing substances live in the layer, networks
  (electrical/plumbing) live in block/tile-entity space, both feed one generic `Overlay`.
- Rust engineers hot/correct loops; Lua *declares* content. Hot paths stay out of Lua.
- Server-authoritative simulation; client reports input, server validates/corrects.
- `CONTEXT.md` holds **intent only**; git holds code state (this distinction is the
  lean philosophy).
