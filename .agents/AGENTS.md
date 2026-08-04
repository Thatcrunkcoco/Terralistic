# Agent Orientation — Terralistic

This file orients any agent working in this repo.

## ⚠️ MANDATORY: Read CONTEXT.md First

**At the start of EVERY session, read `CONTEXT.md` (repo root) before doing anything
else.** It holds the current intent: the `## Next Up` item (your starting task) and
`## Decisions` (conventions to respect). Technical *code* state is in **git** — do
not expect CONTEXT.md to contain it; consult git (`git status`, `git diff`, `git log`)
and the source for that.

## Session Continuity Loop

1. **Start** → read `CONTEXT.md` (mandatory). Start from `## Next Up`.
2. **Work** until done or the user says pause/summarize.
3. **Summarize** → update `CONTEXT.md` **intent sections only** (Next Up + Decisions)
   and append a thin 2-4 line dated log to `session_summaries/<YYYY-MM-DD-HHMM>.md`.
4. Next session repeats.

Full instructions: `.agents/recipes/handoff.md`.

## Key Paths

- `CONTEXT.md` — intent/handoff state (READ FIRST; keep lean).
- `session_summaries/` — thin intent-log history (not a changelog).
- `base_game/*.lua` — Lua content/definition layer (blocks, items, recipes, etc.).
- `shared/`, `server/`, `client/` — Rust engine. `shared/` has hot reusable logic
  (scheduler, players, mod_manager, lights, packet/events).
- `build_project/` — build tooling (compile_mod, compile_resource_pack, png_to_opa).
- `README.md` — full architecture + roadmap + known bugs + tests.

## Things To Know

- Rust does heavy lifting; Lua composes content. Keep hot loops out of Lua.
- Server-authoritative simulation; client reports input, server validates.
- Run tests: `cargo test`.
- **Lean rule:** never bloat CONTEXT.md with code state — git owns that.

## Memory

`Memory` and `Chatrecall` extensions are available for cross-session durable facts.
`CONTEXT.md` is the canonical intent store for this repo.
