# Spec hot-reload — interactive edit→see loop

The fourth high-value UX item: an author edits a spec and the chart updates without
a manual kill + re-run. (Stacked on the `ux/do-first-render-polish` work — PR #7.)

## What shipped

`brightfield-app`:

- **`run_pipeline` now returns `Result`** instead of calling `process::exit` on
  parse/analysis/engine/empty-marks failures. This is the prerequisite: a mid-edit
  save that is momentarily invalid must not kill the window. `main` turns the
  *initial* error into a clean `exit(1)` (CLI behaviour unchanged — verified: exit 1
  on a bad spec, 0 on a good one).
- **`spawn_spec_watcher`** (macOS window path): a detached foreground task polls the
  spec file's mtime every 300 ms. On a change it re-runs the (blocking) pipeline on
  the **background executor** — returning only the `Scene` (which is `Send`) so the
  result crosses the thread boundary — then on the main thread swaps it into the live
  `ChartState` (`set_scene` + `notify`) and `refresh_windows()` to repaint, all in one
  update cycle. A failed reload keeps the last good chart and prints the error.

## Verification — and its limit

Fully verified: compiles clean; `cargo test --workspace` green including a new
`run_pipeline_returns_err_on_bad_spec_instead_of_exiting` test (which would kill the
test process if the `exit` were still there); CLI exit codes; headless render
unaffected.

**Not runtime-verifiable here** (no display/Metal): whether the chart actually reloads
on save. As with the mouse-wiring, the GPUI async wiring was put through an adversarial
review against the gpui source. Verdict: **hot-reload will work at runtime** — the
background pipeline + `Send` scene hand-off is sound (no deadlock, no Send violation,
no busy-spin, no quit-panic), and `refresh_windows()` drives `ChartView::render →
ChartElement::paint` to re-read the new scene via the macOS display link.

**Confirmed working in a real macOS window by the user on 2026-06-29** — editing
`examples/scatter.yaml` and saving updates the chart without a restart. The
adversarial review's positive verdict held up at runtime.

## Review fixes applied

- Wrap the pipeline call in `catch_unwind` so a *panicking* (not just `Err`-returning)
  mid-edit spec keeps the last good chart instead of crashing the window.
- Fold the repaint into the swap update (`app.refresh_windows()` inside the same
  `cx.update`) so the refresh effect flushes deterministically rather than relying on
  the next display-link tick.

## Deferred follow-ups (from the review)

- Reload rebuilds the scene at a hardcoded 640×480; once window-resize is wired into
  `ChartElement`, read the dimensions from `ChartState` instead. → card 0013 / 0010.
- macOS doesn't quit on last-window-close, so after a close the detached watcher keeps
  polling + running pipelines into the void. Stop the loop when the window/entity is
  gone (e.g. exit on `WeakEntity` upgrade failure). Harmless but wasteful.
