# Session Summary — 2026-08-04 (Cartoon fluid renderer + performance + wall containment)

## What happened

Implemented the **previously-deferred ONI-grade cartoon fluid renderer** (the "field
texture" approach from the fidelity roadmap option [1]) for gases *and* liquids, then
optimized it and made fluids respect solid-block boundaries. This was a long visual
iteration with heavy user feedback — the final look is a smooth, gooey, cartoon fluid
that reads ONI / Cult-of-the-Lamb / Paper Mario style.

The user wanted the fluids to look "cool/cartoony" with **minimal manual art** —
procedurally, shader-driven. Every step was user-approved; the user handles **all git**
(no commits were made this session).

## What got built

### A. Field-texture renderer (`libraries/graphics/substance/field_shader.rs` + `renderer.rs`)
The core fidelity leap: instead of drawing one flat quad per tile (which made every
block read as a single flat pixel), we now **bake the visible substance region into an
RGBA GPU texture** (1 texel = 1 tile; R/G/B = gas color, A = fill 255), draw **ONE region
quad**, and let the shader **bilinearly sample the field at fractional-tile coords**. The
GPU's LINEAR filter + an in-shader gaussian blur melts per-tile steps into a smooth
continuous gooey body. The old per-tile quad path (`substance_shader.rs`, v1) is kept but
unused.

- `client/game/substance.rs` — `SubstanceRenderer`: builds **two** RGBA field buffers
  (gas / liquid) from `layer.get_cell` per frame (culled to the visible region, capped at
  `MAX_SUBSTANCE_CELLS = 120_000`), uploads via `glTexImage2D`, and draws one region quad
  per pass through `graphics.render_field`. A per-gas appearance `HashMap` (keyed by gas
  id, ~6 distinct ids) avoids re-locking the registry per cell; `has_gas`/`has_liquid`
  skip a whole pass when that type is absent.
- `renderer.rs` — `render_field()` binds the field program, sets all uniforms, binds the
  field texture on unit 0, draws, then re-binds the passthrough shader (mirroring
  `blur_region` — an invariant: custom shaders must re-bind the default-bound program).
- `VertexBuffer` gained `clear()` and `build_region_quad(screen, world)` (2 triangles,
  white color, world tile coords baked into `tex_pos`, which the fragment shader uses as
  world coords to anchor noise). `rect_array.rs` re-exports `SubstanceTime` (monotonic
  rolling seconds) and a `render_substance` v1 path (kept, unused).

### B. Fragment-shader cartoon look (gas + liquid branches)
- **`sampleField`** — in-fragment gaussian over the field. **5×5 → 3×3** this session
  (see optimization below).
- **Surface wave** — `disp` displaces the sample UV by world-anchored sine octaves
  (`disp.y` up to ~1.2 tiles) for an undulating fluid top.
- **Roil** — continuous `wobble()` slow undulation for body life.
- **Gas branch** (`has_bubbles == 0`): faint (`bright = 0.85 + roil*0.15`), sparse
  drifting sparkle motes. **The user approved this exactly** — it is the reference look.
- **Liquid branch** (`has_bubbles == 1`): "gas mechanics toned up" — richer color
  (`bright = 1.06 + roil*0.18`), +30% saturation, +15% brightness, dense smooth rising
  bubbles. This is the "apply what fixed the gas, to the water, thicker" request.

### C. Chunking / aliasing fix — **continuous noise over per-tile hash**
The recurring "blocky chunking" / "little square blocks standing out" bugs were caused by
**discrete per-integer-cell noise**: `hash21(floor(tile))` snaps the effect to a coarse
per-tile (or 2×2-tile) grid, so squares pop in/out. The fix (applied to **both** gas motes
and liquid bubbles) was to replace every `floor()`-quantized hash with **continuous,
non-quantized noise** — `wobble()`/`sin()` on smooth world coords, never a per-cell hash.
With the same continuous mechanics on both, the gas's aliasing vanished and the liquid's
chunking vanished. `hash21()` was deleted entirely. Gas kept faint; liquid kept vivid.
**This is the code the user said "looks really good."**

### D. Performance (GPU fan spin-up)
The big per-pixel cost was the 5×5 `sampleField` (25 texture fetches) run twice
(gas + liquid passes). Shrank the kernel to **3×3 (9 taps)** — a ~2.8× reduction in the
dominant GPU cost. Also removed per-cell `get_cell()`/`translate_coords`/`Result`
overhead in the CPU scan by iterating the **dense `cells()` slice** with running
`src`/`out` cursors (row stride = map height) instead of per-cell lookups.

### E. Fluids respect solid blocks (wall containment)
Fluids previously painted over and wobbled past neighboring blocks. `SubstanceRenderer`
now takes `&Blocks`; `build_solid_mask` flags cells that are a **solid (non-ghost) block
plus a 1-tile orthogonal ring** around them. Logic:
- **CPU:** the field scan skips (alpha stays 0) any substance cell inside the solid
  mask, so no liquid/gas paints over walls — the discrete edge lands at the boundary.
- **Shader:** a second 1-channel `block_mask` texture (same 1-texel/tile layout, LINEAR)
  is sampled; the surface-wave `disp` is scaled by `(1 - block)`, so the animation
  **freezes to a stop** at block edges while running free in open fluid. This is the "stop
  the animation once it's at the edge next to a block" request — done purely at render
  time, since flow mechanics/containment are not yet built.

## Files touched
- `libraries/graphics/substance/field_shader.rs` (new earlier; main iteration site)
- `libraries/graphics/substance/substance_shader.rs` (v1 per-tile, unuseed superseded)
- `libraries/graphics/substance/mod.rs` — module + re-exports
- `libraries/graphics/mod.rs` — re-exports (`FieldShader`, `SubstanceShader`, `Surface`, `VertexBuffer`, `SubstanceTime`)
- `libraries/graphics/renderer.rs` — `field_shader` field + `render_field()`
- `libraries/graphics/vertex_buffer.rs` — `clear()` + `build_region_quad()`
- `libraries/graphics/rect_array.rs` — `SubstanceTime` + `render_substance` (v1)
- `client/game/substance.rs` — `SubstanceRenderer` (field build, solid mask, uploads)
- `client/game/gases.rs` — `appearance()` helper, removed `is_renderable`/`is_liquid`
- `client/game/core_client.rs` — wired renderer + always-on `request_live_updates(true)`
- `client/game/overlay.rs` — `is_open()` kept public, `#[allow(dead_code)]`

## Verified
- `cargo build`: clean — only the pre-existing unrelated `UI_EVENT_SENDER` static-mut
  warning remains.
- `cargo test`: **83/83 passing.**
- Shaders validated with `glslangValidator` (the standalone `.glsl` files need a `.vert`/
  `.frag` extension to validate; GLSL strings embedded in Rust must avoid unescaped `"`).

## Next Up / follow-ons (not yet scoped)
- **User must rebuild + relaunch** to see the wall-containment + 3×3 kernel (already
  delivered, awaiting final approval).
- **Flow/containment mechanics** are still not built — the fluid sim ignores walls today;
  this session's containment is purely visual masking. When real wall-collision flow is
  added, revisit so the two cooperate.
- **Fidelity roadmap still open** (see `CONTEXT.md`): option [3] finer sim data
  (amount-as-pressure sub-tile), [2] heightfield/surface-wave vertices, [4] post layers
  (glow/blur + specular), and the `SUBTILE` field-resolution bump (more texels per tile)
  if liquid ever needs crisper silhouettes.
- **Per-substance `viscosity`** on `GasType` so liquids level/flow slower than gases.
- **New overlays** (plumbing, electrical, item transport) as `OverlayProvider`s.
- Clean up legacy "pressure" wording in a few test function names/comments (cosmetic).

## Design notes / locked decisions
- Field texture stays at **1 texel/tile** (cheap upload); smoothing is delegated to the
  GPU shader kernel + bilinear filter, NOT CPU pre-blur (a CPU pre-blur v2.6 experiment
  was reverted — the user preferred the GPU-blur gas look).
- **Continuous noise, never per-cell hashes**, for any animated effect that must look
  smooth — the recurring chunking/aliasing root cause.
- Stage order in the render loop: terrain → substance (fluids) → gas overlay → players →
  items → lights → selector → inventory → health.
