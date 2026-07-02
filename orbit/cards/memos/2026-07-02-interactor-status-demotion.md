# Harden — demote over-reported interactor/input statuses

The vocab/runtime alignment pass (`2026-06-30-vocab-runtime-alignment.md`) only
*promoted* truly-working entries and explicitly deferred *demoting* the
over-reported ones to "its own card — needs the guard tests rethought and
accepts added warnings for those kinds." This is that card.

## What changed

`crates/brightfield-spec/src/vocab.rs` — flipped 10 rows from `Implemented` to
`Unimplemented`. All are **parsed but unwired**: helper code exists and is
unit-tested, but nothing on the live loop consumes the interactor/input.

- **`nearest` / `nearestX` / `nearestY`** — `find_nearest` (`nearest.rs`) has zero
  production callers; the hover state is built with `nearest: None` hardcoded
  (`chart_state.rs`) and never resolves a hit. Reverses ifb ac-09 (and honestly
  retires ac-10, whose "hover handler calls find_nearest" was never wired).
- **`pan` / `panX` / `panY` / `panZoom` / `panZoomX` / `panZoomY`** —
  `NavigationConfig::from_interactor_kind`, `apply_pan`, `apply_zoom` and
  `ChartState::set_navigation` are called only from tests; there is no
  scroll/wheel handler and `NavigationState` is always `None` in production.
  Reverses nav ac-11.
- **`slider`** (`InputKind`) — `SliderBinding`/`SliderState`/
  `commit_slider_release` are referenced only in their own tests; the app renders
  no input widgets. Reverses rpw3 ac-14 (which framed its guard test as
  revert-prevention — deliberately reversed here). Re-promote when the param
  coordinator drives re-execution (**card 0005**, reactive parameters).

`highlight` stays `Implemented` (out of scope): the renderer's `HighlightState`
dim/emphasis is genuinely wired, and demoting it would make the curated
`facet-interval.yaml` spec blocking in `curated_preflight`.

## Guard tests (inverted, not deleted)

The three vocab status-pin tests now assert the *demoted* status, so an
accidental re-promotion without wiring fails CI:
`feedback_variant_statuses_after_demotion` (Highlight Implemented, Nearest*
Unimplemented), `input_kind_slider_unimplemented_until_wired`,
`pan_variants_unimplemented_until_wired`.

## Blast radius

None beyond those tests. **No curated conformance spec uses `nearest`/`pan`/
`slider`**, so `curated_preflight`/`registry_integrity`/`deviations.yaml` are
untouched (`blocking()` is Unimplemented-only, but these appear in no curated
spec). The `analysis.rs` slider/panZoom tests are unaffected — parsing still
succeeds (the node keeps its options; only a warning is added) and those tests
assert on `ParamTypeMismatch` / brushable-binding *kind* filtering, not status.
No `examples/` spec uses the demoted kinds (only `intervalX`, which stays
`Implemented`).
