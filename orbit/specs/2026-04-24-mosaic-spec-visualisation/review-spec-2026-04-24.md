# Spec Review

**Date:** 2026-04-24
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-24-mosaic-spec-visualisation/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 3 |
| 2 — Assumption & failure | ambiguous verification wording, cross-crate bridging signal | 2 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [MEDIUM] AC-01 verification uses ambiguous term "inline data source"

**Category:** test-gap
**Pass:** 1
**Description:** AC-01's description states that `MarkData::Inline` returns `EmitError::UnsupportedMark`. However, AC-01's verification says "Session.execute_mark(0) succeeds for a dot mark with inline data source (end-to-end: parse -> emit -> execute -> Arrow)". The word "inline" is ambiguous — it could mean a `MarkData::Inline` variant (which should fail per the AC description) or a data source declared within the mark via `data: { from: table_name }` (which should succeed). A test author reading this verification will not know which to implement.
**Evidence:** AC-01 description: "If the mark has no data or has MarkData::Inline, it returns EmitError::UnsupportedMark." AC-01 verification: "Session.execute_mark(0) succeeds for a dot mark with inline data source".
**Recommendation:** Reword the verification clause to "a dot mark with `data.from` pointing to a loaded table" or similar phrasing that cannot be confused with `MarkData::Inline`.

### [LOW] Implementation notes contradict on column projection

**Category:** constraint-conflict
**Pass:** 1
**Description:** Implementation note 1 says "SimpleLowerer is intentionally minimal — SELECT * FROM view." Implementation note 2 immediately corrects it: "Actually, SELECT columns FROM view where columns come from the mark's channel options." The AC-01 description says SimpleLowerer emits `QueryPlan::Source { table: source }`, which is a table reference — column projection would require wrapping it in a `QueryPlan::Projection`. The notes disagree with each other and with the AC.
**Evidence:** `implementation_notes[0]` vs `implementation_notes[1]` vs AC-01 description (`QueryPlan::Source { table: source }`).
**Recommendation:** Pick one approach and make the AC description match. If SimpleLowerer emits only `QueryPlan::Source`, remove the column-projection note. If column projection is desired, update AC-01 to say it emits a `QueryPlan::Projection` wrapping a `Source`. Given the "minimal" philosophy, `QueryPlan::Source` (i.e., SELECT *) is the simpler choice — projection can be added later.

### [LOW] AC-04 verification is manual-only

**Category:** test-gap
**Pass:** 1
**Description:** AC-04's verification is "The binary compiles. cargo build -p brightfield-app succeeds. A smoke test with a valid spec YAML file opens a window (manual verification)." The compile check is automatable and good, but the "opens a window" part is purely manual. This is acceptable for v2 (first render), but worth noting that there is no automated regression guard for the orchestration pipeline.
**Evidence:** AC-04 verification field.
**Recommendation:** No change required — this is a known gap for a GUI integration AC. Consider adding a headless smoke test in a future card that exercises the parse-execute-render pipeline without opening a window.

### [MEDIUM] ChannelMap::from_mark currently silently drops ParamRef — AC-06 assumes logging infrastructure exists

**Category:** assumption
**Pass:** 2
**Description:** AC-06 requires `from_mark` to log a warning when it encounters a ParamRef. The current implementation (`crates/brightfield-render/src/channel.rs:104-113`) silently skips non-string values with no logging. The spec assumes a logging mechanism (e.g., `tracing`, `log`) is available in `brightfield-render`. If the render crate has no logging dependency, adding one is an implicit prerequisite.
**Evidence:** Current `from_mark` implementation has no `log::warn!` or `tracing::warn!` call. AC-06 verification expects "a logged warning".
**Recommendation:** Confirm that `brightfield-render` has (or will gain) a logging dependency. If not, add `tracing` as a dependency to the constraints or implementation notes. This is a small addition but should be explicit.

### [LOW] No AC covers the canvas()/img() paint mechanism from interview Q5

**Category:** missing-requirement
**Pass:** 2
**Description:** Interview Q5 decided on "CPU readback via canvas()/img() element" as the paint mechanism for bridging Vello into GPUI. No acceptance criterion covers this — the closest is AC-04 which describes the binary and orchestration but not how the scene becomes pixels in the GPUI window. The ChartView/ChartElement rendering path is implicit.
**Evidence:** Interview Q5 decision; AC-04 stops at "opens a window with ChartView" without specifying the rendering mechanism.
**Recommendation:** This is acceptable if the rendering mechanism is considered an implementation detail covered by "manual verification" in AC-04. If the canvas()/img() approach is load-bearing (it is — it determines whether the chart is visible), consider adding a brief note to AC-04's description or implementation_notes clarifying the expected paint path. No new AC needed.

---

## Honest Assessment

This spec is well-structured and covers the integration layer thoroughly. The biggest risk is the ambiguous "inline data source" wording in AC-01's verification — a test author could write a test that accidentally validates the wrong case, creating a false-green gate. The implementation notes contradiction on column projection is minor but could cause a mid-implementation decision point that should have been settled in design. After fixing the AC-01 verification wording, this spec is ready for implementation.
