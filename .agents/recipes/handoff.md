---
name: handoff
description: >
  Session handoff loop for Terralistic. At session start, ALWAYS read CONTEXT.md
  so work picks up where the last session left off. At session end (or when asked
  to "summarize"), update CONTEXT.md with intent only and append a thin dated log.
---

# Handoff Loop

Terralistic uses a **lean** session continuity loop. The goal is to preserve the
~20% that git can't: **intent, in-flight direction, and the explicit next step.**
We deliberately do NOT duplicate code state here — git is the technical source of
truth.

## The Cycle

1. **Start of session** → ALWAYS read `CONTEXT.md` first (non-negotiable).
2. **Work** on the stated `## Next Up` item until it's done or the user pauses.
3. **Summarize** → update `CONTEXT.md` (intent only) + append a thin dated log.
4. **Repeat** in the next session.

## At Session Start (MANDATORY)

1. Read `CONTEXT.md` (repo root) — every session starts here.
2. Treat the `## Next Up` section as this session's starting point.
3. For **technical state**, consult git and source — do NOT expect the handoff to
   contain it.

## When Asked to "Summarize" (end of a working block)

1. Ground the summary in reality: run `git status` + `git diff --stat` and inspect
   recently-touched files. (Code state lives in git, not the handoff.)
2. Update `CONTEXT.md` **intent sections only**:
   - **Next Up**: a crisp, actionable next step (the next session will start here).
   - **Decisions**: conventions/design choices made this session worth keeping.
   Delete the previous Next Up once it's done. Keep it to one screen — bullets only.
3. Append a **thin** dated entry to `session_summaries/<YYYY-MM-DD-HHMM>.md` —
   a short intent log (what happened + next), NOT a code changelog. If git already
   tells the story, keep the log to 2-4 lines.

## When Starting Unrelated Work

If the user gives a fresh task separate from `Next Up`, pivot to it but do not
silently drop pending work — record its completion/state in Next Up when you next
summarize.

## Rules

- `CONTEXT.md` = **intent only**. If you're about to write code-state details here,
  stop — that belongs in git/README.
- Keep it lean. If a section exceeds ~5 bullets, tighten it.
- Git is source of truth for code; `CONTEXT.md` for intent. Neither replaces the other.
