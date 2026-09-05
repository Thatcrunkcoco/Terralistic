# AGENTS.md

## Hard limits

- **Image budget: 30 images max per request (hard provider limit).** Images remain in
  conversation history, so exceeding this permanently bricks the session. When doing
  agentic screenshot/frame diagnostics (`--capture`, frame dumps, PPM/PNG sheet dumps,
  `Read` on images): review only the fed frame or a few sampled frames (e.g. every
  10th), delete consumed frame files from disk, and prefer numeric probes
  (`CAPTURE_WORLD`/`CAPTURE_PROBE`, trace CSV, `--dump`) over visual checks. Never
  `Read` whole frame batches.

## Agent tooling

- **`TESTING.md` is the authoritative manual for the test harness** (capture CLI,
  `--script` DSL, numeric probes, scene recipes, smoke gate). Read it before doing any
  test/validation work; it is what makes "capture frames / drive the game / verify the
  save" possible fully headless and without touching real saves.
