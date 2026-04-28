# Spec Review

**Date:** 2026-04-28
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-28-statistical-marks/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 0 |
| 2 — Assumption & failure | not triggered | — |
| 3 — Adversarial | not triggered | — |

## Findings

(None.)

---

## Honest Assessment

I read the spec cold. All 17 ACs map cleanly to the six approved decisions (D1-D6). Each AC has a specific, testable verification step. Gate ACs (ac-15, ac-16) have non-placeholder verification fields meeting the deterministic length and content rules.

The KDE 2D signature now uses a flat `&[u32]` with `(usize, usize)` shape, consistent with the no-new-deps constraint. The cache cap is fixed at 32 with a dedicated LRU eviction test (gomb_ac11_sql_cache_lru_eviction). The `duckdb_execute_count` test-only accessor is the named ground truth for both ac-11 and ac-12, removing the ambiguity from the prior cycle. The dispatch site verification is now belt-and-braces — both negative (no `_ => DotRenderer`) and positive (find_renderer present in main.rs).

Polynomial/exponential regression rejects loudly with an explicit error message; deferred density variants (DenseLine, Heatmap, Contour, Raster) stay Unimplemented. The CI-band 32-grid sampling and the Silverman-rule defaults are concrete and testable.

No content signals fired (no production deployment, no auth, no data migration). Pass 2 not warranted.

Spec is ready for implement.
