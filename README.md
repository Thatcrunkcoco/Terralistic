# Terralistic

A Terraria-style sandbox game. Terralistic uses a **client-server architecture** with
a Rust engine at its core and **Lua mods** as the content/definition layer. Single-player
runs both the server and client in one process over localhost; multiplayer uses the
same authoritative server for friends to join.

> **Note:** the section below is **high-level creative direction** — the intended
> vision for where Terralistic is heading. It is not a spec of what's implemented
> today. For the current technical state, see the [Architecture](#architecture) and
> [Core Systems](#core-systems) sections further down.

---

## Creative Direction

### The Premise

You are a person who has **crash-landed on an alien planet**. Where the ship comes to
rest, you carve out a small, contained pocket of breathable air — a fragile foothold
against an environment that is not made for you. From there, everything is about
**growing outward**: expanding your base, pumping air and water into new spaces,
pushing into hostile territory, and climbing the ladder of materials, machines, and
bosses that leads first off-world, and eventually to other planets entirely.

### Two Design Pillars

Terralistic blends two distinct pillars of block-based game design:

- **From Terraria — exploration & progression.** A vast, procedurally generated
  world meant to be explored and conquered. Dig deep, travel far, seek out rare
  materials, and face progressively harder bosses that gate access to new tiers of
  gear and content. Progression is driven by *what you go out and find*.
- **From Oxygen Not Included — deep survival engineering.** A rich, systemic
  simulation under the surface. A base is far more than a pretty structure: it must
  *breathe*, *eat*, and *not collapse*. Layered mechanics around gases, liquids,
  temperature, power, and resource loops reward careful base design and planning.

### The Bridge

The defining hook of Terralistic is the connection between the two pillars. The
player is the bridge: you must **explore and fight** to gather what your base needs
to survive, and you must **design a base** robust enough to fuel the deeper
expeditions that progression demands.

Neither pillar can be skipped. Exploration without a capable base stalls; a base
without exploration runs dry.

### The Progression Ladder

The crash-landing framing gives the game a clean, tiered arc — each tier is a new
domain to breathe in, new materials to find, a new set of base systems to master, and
(or shortly after) a boss to overcome:

| Tier | Domain | What unlocks |
|------|--------|--------------|
| **1 — Crash Site** | A small breathable pocket of air | Contained base; manage the air you landed with |
| **2 — Local Expansion** | Tunnels & caves just beyond the pocket | Air plumbing, pumps, sealed doors, gas mining |
| **3 — Hostile Biomes** | Deep caves, flooded zones, heat & toxin areas | Temperature control, fluid transport, pressure suits |
| **4 — The Planet** | The planet's full surface & biomes | Power grid, refining chains, automation |
| **5 — Orbital** | Space station / low orbit | Rocket construction, launch pads, orbital staging |
| **6 — Multi-planet** | Other planets with different atmospheres & fluids | Interplanetary cargo, fluid & item transport, resource export |

### Bosses as Gates

Bosses should be more than damage checks — each one should force a meaningful **base
upgrade**. For example: defeat a boss to unlock the pump, which lets you drain a
flooded biome, which contains the next material, which enables the next rocket
component. In this way, bosses bind the two pillars together rather than standing
apart from them.

### Multi-Planetary Endgame

The long-term horizon is a **multi-planetary game**: a rocket system, item transport,
and fluid transport between planets — each planet effectively a fresh systemic world
of its own, with new atmospheres, new fluids, perhaps new gravity, and their own
bosses and exploration.

### The Player-Facing Loop

In short: **you explore and fight to supply your base; you build a better base to
fuel the deeper expeditions that progression demands.** Start small and contained.
Breathe. Then expand — until the sky is not the limit.

---

## Getting Started

### Build & run the client

```bash
cargo run
```

### Build & run in release mode

```bash
cargo run --release
```

### Run the server (headless / with UI)

```bash
cargo run -- server
cargo run -- server --nogui
```

### Run with debug logging enabled

```bash
cargo run -- --debug
```

> See the [Debug Logging](#debug-logging) section below for what `--debug` does.

---

## Architecture

Terralistic is split into a **Rust engine** and a **Lua content/mod layer**:

- **Rust** owns everything that must run fast or stay correct: the authoritative
  server simulation, physics & movement, chunk rendering, networking & state sync,
  the event/message bus, the type registries, and the per-frame hot loops.
- **Lua** (via `base_game/*.lua`) *declares* what exists and how it behaves: blocks,
  items, tools, walls, recipes, biomes, and commands. It configures content; it does
  not run the tight per-frame loops.

The guiding principle: **Rust does the heavy lifting; Lua composes it.** Keeping the
hot paths in Rust is what lets the game scale to many entities without being
bottlenecked by the scripting layer.

### Multiplayer model

This is a **client-server (host-and-play)** design — one machine runs the
authoritative server while friends connect to it. This is the same model Terraria
and Minecraft use (often informally called "P2P," though it is really a
client-server topology). Single-player is the same code path, just on localhost.

---

## Core Systems

### Tile Entities

A framework for blocks that have state that advances over time (e.g. a furnace that
smelts). Lua declares what a block does (recipes, speed, slots); **Rust runs the
per-tick loop**, stored state in each block's persistent data so it survives
save/load. Includes the furnace as a reference implementation.

### Activity Scheduler

A reusable, generic scheduler (`shared/scheduler.rs`) that tracks which entities are
"active" so a system updates only the working subset each frame rather than
everything. Tile entities use it today; it is designed to be reused by future systems
(mobs, machines, weather) so per-frame cost stays proportional to active work.

### State Sync

Block/tile-entity changes are pushed from the server to clients through a generic
per-block state blob, so dynamic state (like furnace progress) can reach clients for
display without bespoke per-system packets.

### Debug Logging

Runtime trace + crash capture, enabled with the `--debug` CLI flag (see below).

---

## Debug Logging

Pass `--debug` to enable structured runtime logging and crash capture:

```bash
cargo run -- --debug
```

- **Log file:** written to `logs/terralistic.log` in the project directory.
- **What's logged:** our own code at TRACE level (e.g. server/client position
  reconcile, player sync). Third-party crate internals (`message_io`, `mio`,
  `arboard`, `sdl2`) are muted so the log stays readable.
- **Crash capture:** a panic hook records panic messages + backtraces to the log file.
- **Disabled by default:** without `--debug`, nothing is logged and there is no
  runtime overhead — so it is effectively excluded from normal play and release.

It is primarily a development/diagnostic tool, e.g. for investigating client/server
sync issues.

### Debugging native crashes (gdb)

A panic hook cannot catch a **native** crash (SIGSEGV/SIGABRT) that happens inside a
C library called over FFI — e.g. OpenGL or SDL2 — even though the logic that triggers
it is written in Rust. `unsafe` calls into these libraries are outside Rust's safety
guarantees, so a bad call (like freeing a GL resource after its context is destroyed)
can segfault the process with no Rust backtrace.

To diagnose such a crash, run the game under gdb and read the native backtrace:

```bash
cargo build
gdb --args ./target/debug/terralistic --debug
(gdb) run      # play, then quit to reproduce the crash
(gdb) bt       # print the native stack trace of the crash
```

The backtrace shows exactly which `gl::*`/SDL call and which Rust `Drop`/method
triggered it, making the fix straightforward.

---

## Testing

The project has an **Rust unit-test suite** run with:

```bash
cargo test
```

Tests cover the engine's core systems:

- Block type registration & validation (duplicate/empty names, tool references)
- Tile-entity registry, ticking, and recipes
- The reusable activity scheduler
- World-map coordinate handling
- Packet (de)serialization
- Event management
- Player state save/load round-trips
- World-generation terrain-height shaping

---

## Known Bugs

- **Saving while falling** - The player state can be saved mid-fall, causing them to
  respawn/reload in mid-air or at an unexpected position.

---

## Features Roadmap

- **Step up single blocks** - Allow the player to automatically step up
  single-block-high ledges instead of requiring a full jump.
- **Block breaking QoL** - Quality-of-life improvements to block breaking, such as
  hold-to-break and wall breaking.
