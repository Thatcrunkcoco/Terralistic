# Session Summary — 2026-08-28 (Zombie entity: smooth walk animation + motion)

## What happened

Added the first non-player entity — a **procedurally code-drawn zombie** — and then
iterated over three rounds of user feedback ("not smoother", "bodies still pop block by
block") to make its walk read as a smooth, continuous gait rather than a discrete
frame-by-frame / block-by-block pop. All art is generated in code (no PNGs). The user
handles all git (no commits this session).

## Root causes fixed (in order discovered)

1. **Pixel snap in the draw position.** `render()` did
   `(position * RENDER_BLOCK_WIDTH - top_left * RENDER_BLOCK_WIDTH).round()`, snapping the
   sprite to whole pixels each frame. Removed the `.round()` — the `Texture::render`
   transform already accepts `FloatPos`, and NEAREST filtering keeps the crisp pixel-art
   look, so fractional positions now render sub-pixel-correct.

2. **Coarse 4-frame limb animation snap.** The walk was 4 baked frames with limb offsets
   rounded to whole pixels (`(sway * 2.0).round() as i32`), so limbs jumped between four
   discrete poses. Fixed by:
   - raising `ZOMBIE_WALK_FRAMES` 4 → 8 → **16**;
   - generalizing `fill()` to accept **fractional** `FloatPos` coordinates with 4×4
     supersampled coverage (full coverage stores directly; partial coverage alpha-blends),
     so limbs glide through the sine cycle instead of snapping;
   - adding a **body bob** (head/torso/arms rise-fall while feet stay planted).

3. **Walk cycle pacing (the "popping frame by frame" feel).** `animation_progress`
   advanced at 0.005/subtick (1.0/sec) and the frame was `.floor()` over `% cycle`, so a
   full 8-frame cycle took 8 seconds — each frame held a full second, reading as a jump.
   Added `frame_index()` pacing the full cycle once per `WALK_CYCLE_SECONDS = 1.0`.

4. **Body "block by block" stepping (the big one).** The server syncs entity positions
   only **once per second**, and the client hard-overwrote `set_x/set_y` on every sync —
   each snap moved the zombie ~1.5 blocks (its walk speed). The client actually already
   integrates physics every 5 ms (`update_entities_ms`), so the movement was otherwise
   smooth; the 1 Hz sync was undoing it. Added **client-side position interpolation**:
   - `ZombieComponent` gained `target_x`/`target_y` (server-authoritative snapshot target);
   - `client/game/entities.rs` `on_event` records the snapshot as a *target* instead of
     hard-overwriting position (non-zombie entities keep old behavior);
   - `client/game/zombies.rs` `update()` lerps position toward the target each subtick
     (`LERP = 0.15`), so bodies glide and converge to the server position.

## Files touched

- `client/game/zombies.rs` — 16-frame strip, fractional sub-pixel `fill`, body bob,
  `frame_index()` cadence, `WALK_CYCLE_SECONDS`, position lerp in `update()`.
- `shared/entities/entities.rs` — `ZombieComponent` target fields + accessors,
  `spawn_zombie` initializes the target.
- `client/game/entities.rs` — sync writes a target for zombies instead of hard snap.

## Verified

- `cargo build`: clean (only the pre-existing unrelated `UI_EVENT_SENDER` static-mut
  warning remains).
- `cargo test`: zombie tests pass (sheet size 256×24 for 16 frames, corner transparency,
  frame-zero head).

## Next Up / follow-ons (not yet scoped)

- **Zombie AI** — chase the player (currently they just patrol back-and-forth).
- **Collision while walking** — zombies flip at world edges but don't turn around at
  blocks; no pathing/obstacle avoidance yet.
- **Reconcile cadence with speed** — `WALK_CYCLE_SECONDS = 1.0` is a fixed visual rhythm;
  it isn't tied to `ZOMBIE_MAX_SPEED`, so step frequency doesn't yet match ground distance
  covered (feet may look to "slide" at other speeds). Worth deriving frame rate from
  actual velocity.
- **Facing** is derived server-side from velocity only — fine for now, but AI/chase will
  introduce a facing state independent of velocity.

## Design notes / locked decisions

- **Animation state is client-derived, not networked**: the frame comes from a local
  `animation_progress` timer and facing from the synced `velocity_x`. No per-entity
  animation packets — this scales to multiplayer with zero extra traffic.
- **Server stays authoritative**: the client only *lerps toward* the authoritative
  snapshot; it never invents positions. Interpolation (`LERP`) is the smoothing knob —
  raise it for snappier convergence, lower it to reduce overshoot.
- **NEAREST filtering is intentional** (pixel-art consistency); sub-pixel smoothing comes
  from fractional draw coordinates + fractional limb offsets, not texture filtering.
- Procedural zombie art stays deterministic and testable (same frame index → same pixels).
