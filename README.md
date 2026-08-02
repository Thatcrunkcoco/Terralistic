# Terralistic

A Terraria-style sandbox game. Terralistic uses a **client-server architecture** with
a Rust engine at its core and **Lua mods** as the content/definition layer. Single-player
runs both the server and client in one process over localhost; multiplayer uses the
same authoritative server for friends to join.

## Synopsis

Terralistic is a block-based sandbox game that blends two distinct design pillars:
the **world exploration and boss-fight progression of Terraria** with the **deep,
survival-engineering mechanics of Oxygen Not Included**.

- **From Terraria** — a vast, procedurally generated world meant to be explored and
  conquered. Dig deep, travel far, seek out materials, and face progressively harder
  bosses that gate access to new tiers of gear and content. Progression is driven by
  *what you go out and find*.
- **From Oxygen Not Included** — a rich, systemic simulation under the surface. A
  base is more than a pretty structure: it must *breathe*, *eat*, and *not collapse*.
  Expect layered mechanics around gases, liquids, temperature, power, and resource
  loops that reward careful base design and planning.

The player is the bridge between the two: you must explore and fight to gather what
your base needs to survive, and you must design a base robust enough to fuel the
deeper expeditions that progression demands.

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
