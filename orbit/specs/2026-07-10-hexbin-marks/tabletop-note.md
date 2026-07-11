# Tabletop note — hexbin, hexgrid, self-aggregating cell (the density-marks follow-up)

**Date:** 2026-07-10
**Cards in scope:** 0008 (grammar-of-graphics mark library — the staged follow-up slice)
**Output spec:** orbit/specs/2026-07-10-hexbin-marks/spec.yaml

Authored by the driving agent (plugin 0.4.38 blocks model-invoked /orb:tabletop).
This is the follow-up the 2026-07-06 density-marks tabletop carved out and argued
for on four axes (new SQL; binWidth semantics; bin_step convention breaks; the
placeholder swap dance). Ground truth re-verified against main 2026-07-10 by a
ref-pinned recon (the stale-recon lesson).

## Recon deltas since the density-marks staging

- **Hexbin cannot reuse DensityLowerer{TwoD} at all** — the staging note's fourth
  axis understated it. Mosaic's hexbin is *self-aggregating*: the vendored corpus
  uses `fill: {count:}` (flights-hexbin), `fill: {avg: score_value}` +
  `r: {count:}` with `binWidth: $binWidth` (wnba-shots). Aggregate-typed channels
  don't exist in our AST yet; they arrive with this spec and are shared with the
  self-aggregating cell (the reason the two are bundled).
- **The cfr round's renderer seam changes the geometry-threading answer.** The
  staging note worried about "hex size threading past the bin_step convention";
  the cleaner mechanism now is geometry travelling **in-band as reserved
  columns** (`__bf_hex_dx`/`__bf_hex_dy`), which survives live rebuilds for free
  — no GCD recovery, no renderer-attr plumbing for SQL-derived sizes.
- **Contour has no augment_scales** (inherits the trait no-op) — a pre-existing
  gap adjacent to the lattice work, recorded in deferred.
- The placeholder census is exact: 6 engine tests + sql dfir_ac08 + conformance
  preflight + app msv_ac05 + the vocab dmk_ac05 guard (parse.rs dfspec_ac08
  already swapped to voronoi when cell was promoted). Swap target: **geo**
  (stays Unimplemented longest; voronoi is taken).

## Values

**Mosaic parity on the flagship at-scale mark.** Hexbin is the mark Mosaic
demos on 200k-row parquet (flights); it is the reference-project look
(embedding-atlas, RillData density views). Parity means: self-aggregation
(`{count:}`, `{avg:}`), pixel-space `binWidth` (hexes look regular on screen,
as d3-hexbin/Plot draw them), and the vendored corpus specs parsing cleanly.
Second value: **geometry decisions ride existing conventions** — deterministic
ORDER BY (#42 class), reserved `__bf_` columns, zero-anchored Sequential fill
(raster precedent), merge-not-clobber augment_scales.

## Trade-offs

- **Pixel-space binWidth needs the plot's static pixel extent inside the
  lowerer** — the one genuinely new seam. Our plots have fixed pixel sizes
  derivable from the spec, and extents already live in SQL as correlated
  subqueries, so px→data conversion is emitting-time arithmetic; but the
  lowerer must learn the plot extent. Expensive-but-worth-it: pixel-space is
  what makes hexes regular and is Mosaic's semantic. Fallback recorded under
  kill conditions.
- **Aggregate channels land minimal**: `{count:}` and `{avg: col}` only
  (what the corpus uses). sum/min/max only if they fall out free. `r: {count:}`
  size encoding is deferred — it needs a radius scale that doesn't exist.
- **`binWidth: $binWidth` (param-driven) deferred** — `opt_f64` is literal-only
  today; param threading into lowerer SQL is its own increment (wnba-shots
  stays a parse-only citizen).
- **Hexgrid v1 is a fixed light stroke** — stroke/strokeOpacity attrs wait on
  the literal-colour substrate (the contour precedent). The mesh + binWidth
  honour is the capability.
- **The KDE lattice fix rides along but as its own slice/AC** with a scoped
  re-baseline: density.png moves by verified fact; heatmap/contour may move.
  Everything else stays byte-identical. If the slice threatens the rest of the
  round it ships as its own PR.

## Halt conditions

- **Byte-identity gate**: every pre-existing example PNG byte-identical to
  baselines EXCEPT those the lattice AC re-baselines with before/after gallery
  evidence (density-family only). A diff outside that sanctioned set = halt.
- **Vendored corpus must parse** after aggregate channels land — any regression
  in the 54-spec corpus is a halt, not an expectation update.
- **No new dependencies; hex math in plain DuckDB SQL** (the bundled libduckdb
  has no width_bucket — the density lowerer's arithmetic-only precedent holds).

## Escalation triggers

- Threading the plot pixel extent into the lowerer turns ugly (e.g. requires
  changing the MarkLower trait for every lowerer) → surface mechanism options
  (trait default parameter, ContourAttrShield-style wrapper injecting a
  reserved option, assembly-time option injection) before committing.
- Aggregate-channel parsing collides with existing channel forms (literal
  maps?) → surface with the exact corpus lines affected.
- The hexbin SQL's plan shape doesn't fit QueryPlan's IR (Aggregation over a
  computed projection) → surface; extending the IR is a design call, not a
  workaround.

## Kill conditions

- **Claim: pixel-space binWidth is emitting-time arithmetic.** Killed if the
  plot extent genuinely cannot reach the lowerer → pivot: data-space binWidth
  v1 (hexes regular in data units, not screen units), recorded as a deviation
  and flagged to Hugh — NOT silently.
- **Claim: cube-round hex binning is expressible in plain SQL.** Killed by
  DuckDB expression limits or wrong results at edges → pivot: emit finer
  rectangular pre-bins in SQL and hex-assign in Rust render-side (keeps
  aggregation in DuckDB; costs exactness at hex borders) — flagged to Hugh
  before shipping.
- **Claim: the dense lattice fix is safe for heatmap/contour.** Killed if the
  re-baseline sprawls beyond the density family → the lattice slice detaches
  into its own round with its own gallery.

## Verification posture

- Aggregate parsing, lowerer SQL (probe the emitted SQL + execute against
  DuckDB fixtures), renderer geometry/scales, vocab swap, lattice grid shape:
  `verifies: capability`.
- The hexbin example's look and the lattice before/after:
  `verifies: stand-in (real thing is Hugh's gallery eyeball), accepted because`
  the PNG gallery loop is established and closes same-week.

## Budget

2–3 Claude-days: aggregate channels + HexbinLowerer + renderer ≈ 1.5; hexgrid
+ self-aggregating cell ≈ 0.5; swap dance + lattice slice ≈ 0.5–1. Tripwire:
if the SQL hex binning isn't producing correct centres against a hand-computed
fixture by mid-round, escalate per kill condition 2.

## Fix-round addendum (2026-07-11)

The round's major review finding: the hex-ac04 on-lattice claim shipped FALSE
behind a weakened probe. The implementer replaced the spec'd centres-equality
probe with a mesh-only pitch self-check and "confirmed visually" — an eyeball
too coarse for a sub-cell drift; all three review lenses caught it by
execution (rendering and measuring). The fix (raw-anchored domains) surfaced
through a genuine escalation: the first mechanism (analytic recovery from the
occupied-centre-widened domain) was proven impossible by the honest probe
itself, and the design call went to the coordinator with options. Lessons,
now standing: (1) when an AC pins geometric identity, the equality probe IS
the AC — a weaker stand-in test is a spec deviation, not an implementation
detail; (2) the escalation register earns its keep when it fires — the {sql:}
channel-form collision it predicted also shipped un-surfaced and was caught
by review instead.
