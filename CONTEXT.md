# Terralistic — Session Handoff

> **Agent: read this file first.** It captures *intent and direction* only — what
> to do next and decisions-in-flight. For technical *state* (what code exists,
> what changed), rely on **git** (`git status`, `git diff`, `git log`) and the
> source. Git is the source of truth for code; this file is the source of truth
> for *what the human wants next*. Keep it lean — it's a handoff, not a changelog.

_Last updated: 2026-09-04 (frame-capture + scripted-play harness, mostly working)_

---

## Next Up

- **CAPTURE/SCRIPT HARNESS (Tier 2) — built this session, uncommitted.** All change is in
  git working tree; git identity is unset in this repo, so commits were left for the user.
  What works, verified with real captures (agent-reviewed PNGs):
  - `client --test [--hidden] --capture <dir> [--capture-frames N] [--capture-interval N]
    [--capture-duration ms] [--script <file>]`: skips menus, boots the in-process
    singleplayer server on a harness-owned flat seed-123 world
    (`CaptureWorlds/capture.world`, deleted fresh each run; never touches real saves),
    captures rendered frames via GL FBO ReadPixels → PNG + `manifest.txt`.
  - Script DSL (`client/game/script.rs`, see `test_scripts/smoke.script`): `wait`, `press`,
    `chat` (handles the chat anti-echo guard via a `TextInput("")` sentinel), `move`,
    `tp` (client-side teleport; server `/tp` is correctly overridden by client-adopted
    positions), `worldclick`, `shot`, `exit`.
  - Determinism: capture runs disable fps limiter/vsync, pin `real_scale`, and advance
    the sim by a fixed 4 sub-ticks/frame (`FramerateMeasurer` deterministic mode).
  - New server command `/setblock <block> <x> <y>` (base_game/commands.lua).
  - Scratch diagnostics (env-gated, harmless): `CAPTURE_WORLD=<path> cargo test
    dump_capture_world_rows` decodes a .world save and prints a row profile — this
    decoded the coordinate system (see below).
  - **Coordinate reference (decoded): positions are in block units as f32**; the flat
    test world is 512×256 *blocks*, ground top row = **y=180**, player height 3 blocks,
    default spawn (256,170); `RENDER_BLOCK_WIDTH=16` px/unit × `real_scale` (~2) on screen.
  - **Coordinate reference (decoded): positions are in block units as f32**; the flat
    test world is 512×256 *blocks*, ground top row = **y=180**, player height 3 blocks,
    default spawn (256,170); `RENDER_BLOCK_WIDTH=16` px/unit × `real_scale` (~2) on screen.
- **OPEN BUG (first task next session):**
  - ~~`/setblock` mutates nothing~~ **RESOLVED (2026-09-05): it works.** Root causes,
    both in my own harness, not the game:
    1. `run_capture_client` spawned a *second* private server (that one saved a
       pristine world over the played one at shutdown) — fixed by letting
       `PrivateWorld::new` own the server thread exclusively;
    2. the chat anti-echo guard (`waiting_for_t`) ate the first injected `TextInput` —
       fixed by sending a sentinel `TextInput("")` first (mirrors real typing);
    3. the capture loop exited at window-close, killing the server thread before its
       world save — fixed by pumping the menu stack until `should_close()` (the
       shutdown/save happens in stack updates after the window closes).
  - Verified by save-dump: `/setblock stone 16 5` persisted (`id12 at (16,5)` in
    `capture.world`); earlier "invisible stone" readings were reading rows 150-256
    and the block was at row 5 (world y=0 is the sky, 180 the ground).
  - **worldclick** got an adaptive steering loop in `core_client.rs` (`world_click_steering`):
    the script parks the mouse override near the target; the game loop nuzzles it until
    `BlockSelector::get_selected_block` matches the requested block, then injects
    press/release — immune to any residual world-unit mapping drift. Still needs one
    confirmed end-to-end block *break*test; screen-coords mouse override + menu clicks
    are verified working (red outline test).
  - Chat replies/commands fully verified round-trip ("Unknown command", "Set block").



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
  - **Tier 1 numeric test harness — DONE.** `server` CLI grew diagnostic flags:
  `--trace <csv> --trace-ms N` (entity trajectory rows + gas fnv-checksum/per-type-total
  rows, schema `#trace v1`), `--dump <json>` (deterministic end-state snapshot incl.
  `payload_fnv64` golden value), `--duration <virtual_ms>` (auto-stop on simulated time,
  tick-exact). Sim-flag runs use their own world save (`server_data/trace.world`) so they
  never mutate the real multiplayer world. Validated live: zombie x advances exactly at
  1.5 blocks/s across rows; gas totals/fnv constant in the sealed test rooms across the
  whole run.
  - **Bit-exact golden-state tests** need a deterministic stepping loop (today sub-ticks
    fill from wall clock, so cross-run/cross-machine dumps differ slightly near the stop
    edge). In-process nogui `Server` golden tests are the stretch goal.
- **Zombie bake fixes (visual round):** exposed via a new agent sheet-inspection
  hook —`CAPTURE_SHEET=<dir> cargo test dump_zombie_sheet` writes the full 16-frame
  walk strip as PPM at 8x zoom, which the agent reads/converts to PNG and reviews. Fixed:
  straight-alpha partial-coverage compositing (no more gray ghosting), feet anchored to
  their legs (was sliding out from under), leg swing 3→2 (stays under torso), arms now
  pump vertically at fixed x in opposite phase (horizontal slide detached them).
- **Capture harness (next milestone):** frame-capture mode (hidden window,
  png crate, CLI camera) → scripted keyboard play → mouse actions later. Screenshots /
  your video drops also feed my multimodal analysis in-session.

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
