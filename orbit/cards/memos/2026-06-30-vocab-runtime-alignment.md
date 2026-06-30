# Harden — vocab/runtime status alignment (card "harden the render")

The vocab `status()` (Implemented | Planned | Unimplemented) drives a
`ParseWarning::Unimplemented` on every parse. It had drifted from reality:
genuinely-working marks and interactors were still labelled `Unimplemented`, so
**every run printed false warnings** — `dot`, `line`, `bar`, and (since
cross-filter shipped) `intervalX/Y/XY`. This pass aligns the labels with what
actually works.

## The rule applied

**`Implemented` == affects end-to-end output in the currently-shipped pipeline.**
A mark qualifies only with BOTH a registered renderer (`default_renderers`) AND
a working lowerer (`default_lowerers`, not the `DefaultLowerer` fallback). An
interactor qualifies only if a live event path mutates rendered output. Unwired
helper code, renderer-without-lowerer, and parse-only acceptance do NOT qualify.

(An audit workflow rated every vocab entry YES/PARTIAL/NO end-to-end; only YES
entries were promoted.)

## Promoted (7)

- **Marks:** `dot`, `line`, `barX`, `barY` → Implemented (renderer + SimpleLowerer;
  render geometry today — the most-seen false warnings, every basic spec).
- **Interactors:** `intervalX`, `intervalY`, `intervalXY` → Implemented (the live
  cross-filter path: brush → `commit_brush` → `propagate_selection` → re-render).

## Guard tests repointed (3)

Three tests asserted "an Unimplemented mark warns / flows through" using `dot`
or `line` as the fixture. Repointed to marks that are *genuinely* Unimplemented
so the tests keep proving the same thing:
- `parse.rs` `dfspec_ac08` — fixture `dot` → `cell`.
- `conformance/identity.rs` `dfconf_identity_status_flows_through` — `Line` → `Rect`.
- `conformance/support.rs` `dfconf_preflight_reports_unimplemented_mark_only` —
  `line` → `rect`.

## Deliberately NOT changed

- `dotX/dotY/circle/lineX/lineY` — PARTIAL: renderer exists but no lowerer, so
  they're dropped before render. Stay Unimplemented. (A one-line lowerer each
  would promote them — a cheap mark-breadth follow-up.)
- `nearest*`, `highlight`, `pan*`, `panZoom*` (interactors) and `slider` (input)
  are currently labelled **Implemented but don't work end-to-end** (unit-tested
  helpers never wired into the live loop). This pass only *promotes* truly-working
  entries; **demoting** these is deferred — it would *add* warnings for their
  users and break the `vocab.rs` guard tests, so it needs its own card.
- The 47 inert marks (rect/rule/tick/text/geo/hexbin/…) correctly stay
  Unimplemented (no renderer, no lowerer).

## Known cosmetic drift (not fixed here)

Several sites hardcode `status` fields independent of `vocab::status()`
(`ast.rs`, `channel.rs`, `lower.rs`, `layout.rs`). They don't read `status()`, so
tests stay green, but they now encode stale values. Optional cleanup.

## Verification

`cargo test --workspace` green. `examples/crossfilter.yaml` (dot + intervalX) now
parses with **no** false `Unimplemented` warnings and renders at 100% coverage.

## Follow-up

- Demote the over-reported `nearest*/highlight/pan*/panZoom*/slider` (own card —
  needs the guard tests rethought and accepts added warnings for those kinds).
- Register lowerers for `dotX/dotY/circle/lineX/lineY` to actually implement them.
