# Terralistic — Session Handoff

> **Agent: read this file first.** It captures *intent and direction* only — what
> to do next and decisions-in-flight. For technical *state* (what code exists,
> what changed), rely on **git** (`git status`, `git diff`, `git log`) and the
> source. Git is the source of truth for code; this file is the source of truth
> for *what the human wants next*. Keep it lean — it's a handoff, not a changelog.

_Last updated: 2026-08-03_

---

## Next Up

- Handoff loop **validated** (cold-start pickup works). Next: pick a real feature task:
  - **Step up single blocks** — auto step-up single-block ledges instead of a jump.
  - **Block breaking QoL** — hold-to-break and wall breaking.
  - **Fix bug:** "Saving while falling" — player can spawn mid-air if saved mid-fall.

## Decisions

- Rust engineers hot/correct loops; Lua *declares* content. Keep hot paths out of Lua.
- Server-authoritative simulation; client reports input, server validates/corrects.
- `CONTEXT.md` holds **intent only**; git holds code state (this distinction is the
  lean philosophy).
