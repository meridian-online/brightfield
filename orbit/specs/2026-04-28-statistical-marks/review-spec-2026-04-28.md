# Spec Review

**Date:** 2026-04-28
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-28-statistical-marks/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 3 |
| 2 — Assumption & failure | content signals + 1 MEDIUM finding | 2 |
| 3 — Adversarial | not triggered | — |

## Findings

### [MEDIUM] Constraint contradicts implementation note (ndarray vs no-new-deps)

**Category:** constraint-conflict
**Pass:** 1
**Description:** Constraint #8 states "No new dependencies (no nalgebra) — convolution and OLS analytics in pure Rust". But ac-02 declares the kde_2d signature as `kde_2d(bins: &Array2<u32>, ..., bin_size: (f64, f64)) -> Array2<f64>` and an implementation note explicitly says "Optional: ndarray dependency for the 2D path; or a flat Vec<f64> with explicit row-stride indexing." `Array2` is the canonical ndarray type. Either ndarray is allowed (relax the constraint) or the AC signature should be flat-Vec-only.
**Evidence:** spec.yaml line 22 (constraint), ac-02 description (`Array2<u32>`), implementation_notes line referencing ndarray.
**Recommendation:** Pick one. Recommend: hold the no-new-deps constraint and rewrite ac-02 to use a flat `Vec<u32>` with `(width, height)` shape parameters. The 2D Gaussian is separable — convolve rows then columns — so a flat Vec with stride math is actually simpler than an Array2.

### [LOW] ac-11 cache eviction is under-specified

**Category:** test-gap
**Pass:** 1
**Description:** ac-11 says "capped LRU with a small fixed cap (e.g. 32 entries — implementer picks the cap)". No AC verifies the cap is actually enforced. A buggy implementation that grows unbounded would still pass ac-11 and ac-12. Decisions doc names "capped LRU is the recommended starting policy" — capping is the policy, so it should be tested.
**Evidence:** ac-11 description, decisions.md D5.
**Recommendation:** Either (a) add an explicit verification that inserting >cap distinct SQL strings causes the oldest to evict (a one-line test), or (b) explicitly accept that LRU enforcement is a non-functional detail and note it under exit_conditions as "best-effort, not a tested gate". (a) is cheap; recommend (a).

### [LOW] ac-12 verification mechanism is ambiguous

**Category:** test-gap
**Pass:** 1
**Description:** ac-12 verification asks to "record DuckDB execute count" but doesn't say how. The Session API doesn't expose an execute counter (verified by reading crates/brightfield-engine/src/lib.rs surface). An implementer might (a) add a private `execute_count` counter on Session for testing, (b) compare batch identity, (c) instrument DuckDB itself. Without a steer, the test could silently fall back to "did the convolution change?" which doesn't actually prove the cache hit.
**Evidence:** ac-12 verification field; absence of counter API in Session today.
**Recommendation:** Add a steer: "preferred mechanism: a `pub(crate) fn duckdb_execute_count(&self) -> usize` test-only accessor on Session, incremented inside execute_mark just before the DuckDB execute. Compare counts before/after the second execute_mark." This makes the test deterministic.

### [LOW] ac-10 dispatch site location may have shifted post-card-0006

**Category:** assumption
**Pass:** 2
**Description:** ac-10 description hedges with "current line range circa 98-108, may have shifted post-card-0006". The implementer must locate the dispatch site themselves. That's normal, but the AC verification "rg the codebase for the literal `_ => DotRenderer` pattern; assert zero matches outside test code" is a stronger gate than "edit lines 98-108" — good. However, if the dispatch site no longer uses a flat match (e.g. card 0006 already restructured it), the implementer might silently land a no-op. Recommend the verification adds: "AND the dispatch site invokes find_renderer (rg for `find_renderer(`)".
**Evidence:** ac-10 description, drive instructions.
**Recommendation:** Strengthen verification: combine the "no `_ => DotRenderer` pattern" check with a positive assertion that `find_renderer(` appears in main.rs.

### [LOW] No cross-platform note for KDE truncation/numerical reproducibility

**Category:** content-signal
**Pass:** 2
**Description:** Conformance snapshots (ac-13) capture emitted SQL strings. They do NOT capture rendered output (rightly — Vello scenes aren't easy to snapshot). But the Gaussian kernel is implemented in CPU floats with truncation at ±3σ; small floating-point variation across macOS/Linux/Windows could yield tiny differences in convolved density. This isn't a blocker today (the snapshot is SQL only) but worth noting that future render-snapshot work should plan for tolerance.
**Evidence:** ac-13 (snapshot scope), CLAUDE.md (cross-platform target macOS/Linux/Windows).
**Recommendation:** Add an explicit note in implementation_notes that snapshots capture SQL only, not rendered scene; future scene snapshots will need tolerance-based comparison. Non-blocking — purely documentation.

---

## Honest Assessment

The spec is well-structured and ACs are testable with one structural exception (the ndarray/no-new-deps contradiction). All 17 ACs map directly to the six approved decisions; the test prefix `gomb_` is consistent with project convention; the AggregateScalar IR variant and the dual-renderer split are coherent with the existing code surface.

The biggest real risk is the constraint-vs-AC mismatch in ac-02 — if the implementer reads the constraint they pick flat Vecs; if they read the AC signature they pick ndarray. That's a 50/50 coin flip on a structural design decision, and exactly the kind of thing spec review should catch. Fix that, tighten the two cache-related ACs (ac-11 cap test, ac-12 counter mechanism), and the spec is ready for implement.

The 95% CI t-distribution approximation (n>=30 → 1.96) is acceptable for the operating regime; the "rare small-n regression" edge case is not an Mosaic corpus concern.

No deepening triggers fired beyond the ndarray contradiction. Pass 3 not warranted.
