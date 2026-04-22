# Spec Review

**Date:** 2026-04-22
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-22-gpu-accelerated-mark-rendering/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 2 |
| 2 — Assumption & failure | not triggered | — |
| 3 — Adversarial | not triggered | — |

## Findings

### [LOW] ChannelMap type referenced but not yet in codebase

**Category:** assumption
**Pass:** 1
**Description:** ac-02 references "ChannelMap" as an input to scale inference and ac-03/04/05 MarkRenderer signatures. The interview traces this to card 0008 Decision 2, but `ChannelMap` does not exist anywhere in the current codebase (grepping `crates/` returns zero hits). The implementation will need to define this type as part of this card's work.
**Evidence:** `grep -r "ChannelMap" crates/` returns no results. The interview says "ChannelMap (card 0008, Decision 2): typed channel extraction from mark options" but card 0008 has not been implemented.
**Recommendation:** No spec change needed. The implementation notes already describe the MarkRenderer signature including ChannelMap. The implementing agent should define ChannelMap within brightfield-render (or brightfield-spec if it's a parsing concern) as part of ac-02/ac-03 work. This is implicit in the card's scope.

### [LOW] No AC for font loading or text rendering infrastructure

**Category:** missing-requirement
**Pass:** 1
**Description:** ac-06 (axis labels) and ac-07 (legend text) both depend on Vello text rendering via `draw_glyphs()` and `skrifa` font shaping. The implementation notes mention "load system default sans-serif at startup" but no AC explicitly covers font loading. If font loading fails silently, both ac-06 and ac-07 would produce blank text.
**Evidence:** Implementation notes: "Font loading: load system default sans-serif at startup for axis labels and legend text via skrifa". No AC with id covering font loading or fallback behaviour.
**Recommendation:** No spec change needed for v1. Font loading is an implementation detail that will naturally surface during ac-06 work (axis label rendering). If it fails, ac-06's verification ("verify scene contains stroke operations for ticks and grid lines" plus the implicit text assertions) will catch it.

---

## Honest Assessment

This spec is well-structured and ready for implementation. The 11 ACs cover the full rendering pipeline from scale inference through mark rendering to GPUI integration. The two-crate split (brightfield-render headless, brightfield-ui GPUI shell) is the right architecture decision — it makes 8 of 11 ACs testable without a window. The main implementation risk is the Vello/wgpu dependency chain on Apple Silicon (version pinning, Metal backend compatibility), but that's a build-system concern rather than a spec gap. The ChannelMap dependency on card 0008 is a minor coordination point — the type definition is straightforward and can be defined inline within this card's scope.
