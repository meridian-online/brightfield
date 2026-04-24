# Spec Review

**Date:** 2026-04-22
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-22-interactive-feedback/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 1 |
| 2 — Assumption & failure | not triggered | — |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [LOW] find_nearest function signature references ScaleSet but ac-02 says "scans RecordBatch rows via ScaleSet"
**Category:** assumption
**Pass:** 1
**Description:** ac-02 describes `find_nearest()` scanning RecordBatch rows "via ScaleSet" to resolve pixel positions, and the interview Q1 shows a five-parameter signature (`cursor, batch, channel_map, scales, mode`). The AC verification tests cover cursor-on-point, cursor-far-away, and NearestMode axis filtering, which is thorough. However, the AC description omits `ChannelMap` as an input — the function needs it to know which columns map to x/y. This is a documentation gap rather than a test gap, since any working implementation will necessarily accept `ChannelMap`.
**Evidence:** Interview Q1 signature: `find_nearest(cursor, batch, channel_map, scales, mode)`. ac-02 description mentions only "RecordBatch rows via ScaleSet".
**Recommendation:** Cosmetic — no action required. The implementation will naturally include `ChannelMap` in the function signature to resolve column names. The tests described in ac-02 are sufficient to catch a broken implementation.

---

## v1 Review Resolution Check

The v1 review (review-spec-2026-04-22.md) raised 6 findings. The v1.1 spec addresses all of them:

1. **TooltipElement GPUI rendering (MEDIUM)** — Resolved. Implementation note 1 explicitly defers `TooltipElement` to a follow-up card. ac-08 is now clearly scoped to data extraction only.

2. **Hover-to-nearest integration (MEDIUM)** — Resolved. ac-10 added: `InteractionState::Hovering` gains optional `NearestHit` field; hover handler path calls `find_nearest`.

3. **render_interpolated default impl drops highlight (LOW)** — Resolved. Constraint added: "render_interpolated() default impl must forward highlight parameter to the updated render() signature."

4. **HighlightState trait bounds (MEDIUM)** — Resolved. Constraint specifies `Box<dyn Fn(usize) -> bool + Send + Sync>` and ac-03 description matches.

5. **prev_positions insufficient for bar interpolation (MEDIUM)** — Resolved. Constraint scopes mark-level interpolation to DotRenderer only. Implementation note 3 defers bar/line interpolation with rationale. Implementation note 4 acknowledges `Vec<(f64, f64)>` suffices for dots and flags a future `Vec<MarkPosition>` enum.

6. **Highlight/dim opacity fade animation (LOW)** — Resolved. Constraint states highlight/dim snap is immediate (0ms); implementation note 2 explicitly defers fade animation.

---

## Honest Assessment

This spec is ready to implement. The v1.1 revision addressed every finding from the v1 review, either by adding acceptance criteria (ac-10 for hover state), tightening constraints (Send + Sync bounds, highlight forwarding, DotRenderer-only interpolation), or explicitly deferring scope (TooltipElement, fade animation, bar/line interpolation). The 10 ACs cover the full render-crate surface — nearest-point resolution, highlight state, mark rendering with highlight, scene integration, transitions, tooltip data extraction, vocab status, and hover state enrichment. Each AC has concrete verification criteria testable at unit level. The only finding is a cosmetic omission of `ChannelMap` in ac-02's description, which will not affect implementation.
