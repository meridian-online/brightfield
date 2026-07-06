# Tabletop note — Legend click-to-filter (closed design space)

**Date:** 2026-07-05
**Card:** orbit/cards/0009-multi-view-dashboard-composition.yaml (scenario "Legends participate as interactors")
**Output spec:** orbit/specs/2026-07-05-legend-click-filter/spec.yaml
**Mode:** closed-space — the design was fixed by a code recon (main @ 29fb46f) rather than an interview; every load-bearing claim below carries file:line evidence in the recon (relayed 2026-07-05, this session).

**What good looks like** (author's seat): I bind a legend to a selection — `legend: color as: $sel for: scatter` — and clicking a category swatch in the live window filters every view that consumes `$sel`, exactly like brushing does; clicking the same swatch again puts everything back; the legend itself never greys out its own source plot.

## Values

**Reuse the proven dispatch chain end-to-end.** `point_predicate` → `SelectionValue::Text` quoting (#31) → `Session::propagate_selection` → absorb/rebuild is untouched machinery; the feature adds only a producer binding, hit geometry, and one coordinator entry point. Anything that forks that chain is scope creep.

## Locked picks

- **Contributor path = the `for:` plot's node path.** This makes `compile_selection`'s string-equality self-exclusion drop the legend's own source plot, which is simultaneously the right semantic (a legend is a control, not a filtered view) and the thing that keeps the launch-time colour-scale snapshot valid — **no shared-mutable-scale plumbing needed** (the 0016 deferral stays deferred).
- **Toggle semantics v1: single-select toggle.** Click a swatch → `col = 'category'`; click the same swatch → clear; click a different swatch → switch; click empty panel → clear. Coordinator tracks one `Option<category>` per legend binding. No multi-select (shift) in v1.
- **`Scale::Colour` only.** Sequential gradient legends have no discrete entries — no listeners, inert. Display-only legends (no `as:`) stay listener-free.
- **Geometry via an additive helper.** `legend.rs`'s five layout constants are private; a new pure `pub fn swatch_entry_rects(...)` reuses them. The 0016 render-freeze narrows to: **render functions byte-unchanged; additive helper allowed** (duplicating the constants outside legend.rs would drift silently — worse).
- **`as:` on a legend becomes a producer, not a subscriber.** The existing analysis arm that registers legend option param-refs as subscribers must skip the selection binding key (today it wires `as: $sel` backwards).

## Halt conditions

- Any pre-existing example PNG byte-diffs → halt, revert (unchanged house gate; this feature has no composite-path changes).
- The `legend.rs` diff exceeding one additive pure helper + its test → halt (kill condition 1).

## Escalation triggers

- The producer-binding analysis can't resolve the colour column from the `for:` plot's first mark (Fill/Stroke channel absent or ambiguous for some mark family) → surface the mark family list, propose scoping v1 to marks with an explicit colour channel.
- Coordinator liveness/rebuild changes ripple beyond `new()`'s guard + the new commit path → surface diff, propose narrowing.

## Kill conditions

1. **Claim: swatch hit-rects are reproducible from legend.rs constants via an additive helper.** Killed by layout logic that can't be factored without touching render fns → pivot: legend.rs exports entry geometry computed *inside* render (heavier amendment, needs a fresh review of the freeze).
2. **Claim: self-exclusion via contributor-path equality keeps the scale snapshot valid.** Killed by the source plot's scales rebuilding anyway (e.g. another selection also filters it, shrinking categories) → accepted degradation for v1: stale swatch set until restart, same class as the recorded hosted-legend-refresh deferral; revisit with the shared-scale plumbing.
3. **Claim: the existing rebuild loop needs no changes for legend-triggered dispatch.** Killed by legend dispatch needing bespoke rebuild ordering → pivot: reuse commit_brush's exact absorb loop by refactoring it into a shared private fn.

## Verification posture

- Binding analysis, colour-column resolution, subscriber-trap fix: `verifies: capability` (spec-crate unit tests).
- Entry-rect geometry: `verifies: capability` (pure helper vs render constants).
- Dispatch/toggle/self-exclusion/liveness: `verifies: capability` (crossfilter_integration.rs drives the real session end-to-end, matching the existing point-toggle tests).
- The in-window click gesture and visual response: `verifies: stand-in (real thing is clicking the swatch in the macOS app), accepted because GPUI event dispatch needs the window` — backed by the hitbox/hit-test unit coverage.

## Budget

1 Claude working day (recon is done; five files + tests). Tripwire at day 2 → drop toggle-off to last-click-wins and ship.

## Hot-wash

- recurred: the 0016 deferral (shared-scale plumbing) shaped this design from the outside — the contributor-path pick exists to avoid waking it.
- surprised: `as:` on legends is already parsed but wired *backwards* (subscriber, not producer); the colour column was already reachable via ChannelMap.
- meta: recon-then-closed-space-note is fast and honest when the machinery is this mature — the interview would have had nothing to decide that the code didn't already decide.
