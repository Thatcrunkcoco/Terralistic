# Testing & Validation Recipes

This documents the agent-facing test harness: how to run the game with
scripted input, capture rendered frames, and verify world state numerically.
Everything here works headless (`--hidden`) and never touches real saves.

## Design & intent

The harness lets a multimodal coding agent **see and act on the real game**
instead of guessing from code or asking humans for screenshots:

- **See**: rendered frames are read back from the offscreen GL framebuffer
  (`GraphicsContext::capture_frame_to_png`) and written as PNGs the agent can
  review in-session. This is the only ground truth for "does it look right".
- **Act**: a script DSL injects synthetic input at the single `GraphicsContext`
  event choke point (event queue + key-state map + mouse override). Menus,
  chat, inventory, world interaction all go through the same accessors real
  input uses, so scripted play exercises the real game — no fake server, no
  special-case paths.
- **Verify numerically**: world saves are decoded by a scratch test
  (`dump_world_save`), and the sim itself is traced/dumped headless via the
  server CLI. Numeric probes check behavior (movement speed, conservation,
  break/place persistence) without spending the agent's image budget.
- **Safety invariants**: harness runs always use the flat seed-123 test world
  saved to harness-owned files (`CaptureWorlds/capture.world`, deleted fresh
  each run; `server_data/trace.world` for sim traces). Real singleplayer and
  multiplayer saves are never touched.
- **Determinism contract**: capture runs disable the fps limiter/vsync, pin the
  UI zoom, and advance the sim by a fixed 4 sub-ticks per frame. The server
  still ticks on wall clock, so captures are *visually* stable but not
  bit-exact across runs; bit-exact golden frames need a deterministic stepping
  loop on both sides (deferred — see CONTEXT.md).
- **Key code locations**: `client/game/capture.rs` (run driver + `FrameCapture`),
  `client/game/script.rs` (DSL parser/driver), `core_client.rs` (per-frame hooks:
  script pump, teleport, world-click steering, capture/exit) and
  `libraries/graphics/renderer.rs` (event injection, mouse override, PNG
  readback). `test_scripts/*.script` hold worked examples.

Examples of what a developer can now just ask the agent to do:

- *"Capture some frames right of spawn — how are the liquids rendering against
  the box walls?"* → scripted run + `/setblock` scene + PNG review
- *"This zombie animation looks a little off"* → capture while walking, or
  dump the bake sheet (`CAPTURE_SHEET=... cargo test dump_zombie_sheet`)
- *"Does gas conservation still hold with a wall removed?"* → `--trace/--dump`
  numeric check, no images involved
- *"Sea-level looks like it leaks into the house area"* → capture + save probe
- *"Build a torch-on-stone scene at 100,150 and screenshot it"* →
  `/setblock` + `tp` + `shot` one-liner

## Capture + script run (the main tool)

```sh
cargo run -- client --test --hidden \
  --capture /tmp/opencode/frames \
  --capture-frames 10 \
  --capture-interval 0 \
  --script test_scripts/smoke.script \
  --capture-duration 40000
```

- `--test`            skip menus; spawn the in-process server on the flat
                      test world (512x256, seed 123, ground row **y=180**)
                      against its own save (`CaptureWorlds/capture.world`,
                      deleted fresh each run). Never touches real saves.
- `--capture <dir>`   write `frame_NNNNN.png` + `manifest.txt`
- `--capture-frames N`    stop the run after N captures
- `--capture-interval N`  capture every Nth rendered frame; `0` = only on
                          script `shot` actions (default 1 = every frame)
- `--capture-duration ms` hard wall-clock timeout for the whole run
- `--script <file>`   scripted play (DSL below)
- `--hidden`          create the window hidden (agent runs; keep it on)

Deterministic mode is automatic when any harness flag is present: fps
limiter/vsync off, UI zoom pinned, sim advanced by a fixed 4 sub-ticks per
frame. The server still ticks on wall clock, so captures are visually
stable but not bit-exact across runs (golden frames remain a stretch goal —
see CONTEXT.md).

## Script DSL (`--script <file>`)

One action per line; `#` comments and blank lines are ignored.

| Action                        | Effect |
|-------------------------------|--------|
| `wait <ms>`                   | real-time pause before next action |
| `press <key> [hold_ms]`       | key/button press (+release after hold, default 30ms) |
| `move <x> <y>`                | move the mouse override to *screen* coords (in pixel units, same as the red block-selection outline) |
| `chat <text>`                 | open chat, type, Enter, **then Escape** so it stops consuming input. A leading `/` runs a server command |
| `tp <x> <y>`                  | client-side teleport of the local player; position units are **1/16 of a block** (spawn = 256,170; ground row 180) |
| `worldclick <bx> <by> <left\|right> [hold_ms]` | steer the mouse until the block selector selects block `(bx,by)` (same coordinate space as `/setblock` — save block indices), then press/release |
| `shot`                        | force a capture on the next frame |
| `exit`                        | close the window and end the run |

Key names for `press`: `a`..`z`, `0`..`9`, `space`, `enter`, `escape`, `shift`, `ctrl`, `alt`, `delete`, arrows, `leftmouse`/`rightmouse`.

## Gotchas learned the hard way

1. **Select a tool before mining.** After spawn no hotbar slot is selected
   → tool_power 0 → blocks never break. `press 1` = pickaxe (slot 0), `press 2`
   = shovel, 3 = hatchet, 4 = hammer.
2. **Tool must match the block**: dirt needs the shovel (power 10), stone
   needs the pickaxe. Mismatched tool = the break silently never starts.
   Default break times: dirt 700ms, stone 1000ms.
3. `chat` closes the chat box with Escape automatically — script actions that
   come right after a `chat` no longer get eaten.
4. Server-side `/tp` is overridden by the client's own authoritative position
   reports; use the client-side `tp` action for moving the *local* player.
5. World y: row 0 is the sky top, row 180 the ground surface, 255 the bottom.
6. The run continues pumping menus after the window closes so the private
   server join+save completes — never hard-kill the process before
   `server stopped.` appears in the log or the world save is lost.

## Numeric ground truth

### World save dump (after any capture run)

```sh
CAPTURE_WORLD=~/.local/share/Terralistic/CaptureWorlds/capture.world \
  cargo test dump_world_save -- --nocapture
```

- default:        `#world 512x256, non-air blocks: N` (conservation check)
- `CAPTURE_PROBE="x,y x,y"`: print block ids at coordinates
- `CAPTURE_COLUMN=x`: first/last non-air row of one column

### Server sim trace (headless, no client)

```sh
cargo run -- server --nogui --test \
  --trace /tmp/opencode/trace.csv --trace-ms 250 \
  --duration 5000 --dump /tmp/opencode/final.json
```

CSV has per-entity positions and per-gas totals + fnv checksums; the JSON
dump carries the canonical `payload_fnv64` golden value. See
`session_summaries/2026-09-04-tier1-trace-dump-harness.md`.

## Scene setups for visual checks

- **Fluids/gas rooms**: they exist in the test world already — the three-cell
  gas room box sits right of spawn (blocks ~13-32, rows ~177-183); a second
  liquid box further right. `tp 250 150` frames it on screen.
- **Custom scenes**: `chat /setblock <block_name> <x> <y>` (also `stone`,
  `dirt`, `grass_block`, `torch`, `wood_planks`…). Place → `wait` → `shot`.
- **Breaking**: `press 1 30` first, then `worldclick ... left 4000`; the
  steering closes on the game's own selection math so no manual pixel math.
- **Menu/UI shots**: omit `--test`; capture the title screen directly
  (`--capture` alone starts at the menus). `move` + `press leftmouse` can
  click buttons, but menu flows are not scripted yet (future work).

## Smoke gate

After touching client/render/server code:

```sh
cargo build && cargo test --quiet
rm -rf /tmp/opencode/frames && stdbuf -oL cargo run --quiet -- client --test \
  --hidden --script test_scripts/smoke.script \
  --capture /tmp/opencode/frames --capture-frames 10 \
  --capture-interval 0 --capture-duration 40000
```

Expect ~7 frames, all server/client logs clean, `server stopped.` at the end.
Review at most 1-2 frames visually; prefer numeric probes to save the agent's
image budget.
