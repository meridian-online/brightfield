# Tabletop note — heatmap, cell, contour (closed design space; hexbin re-staged)

**Date:** 2026-07-06
**Card:** orbit/cards/0008-grammar-of-graphics-mark-library.yaml (scenario "Specialised marks for geo and binned views")
**Output spec:** orbit/specs/2026-07-06-density-marks/spec.yaml
**Mode:** closed-space — recon against main @ 0b971af (ref-pinned; file:line evidence relayed 2026-07-06).

**What good looks like** (author's seat): I point a heatmap at a point cloud and get a smooth density ramp; I point a cell mark at a day×hour table and get a calendar-style grid coloured by value; I add contour and iso-lines trace the same density the heatmap shades — all with the colour schemes I already know from raster, all in the PNG and the window alike.

## Staging decision (flagged to Hugh — inverts the track's named order)

The recon inverted the cost prior. **This spec ships heatmap + cell (pre-aggregated) + contour; hexbin (+ hexgrid + self-aggregating cell) is carved to a follow-up spec.** Four compounding reasons, all evidenced: (1) hexbin needs new, CASE-heavy axial/cube-round SQL while the other three need zero new SQL (heatmap/contour reuse DensityLowerer{TwoD}; cell v1 is pass-through); (2) Mosaic's `binWidth` is pixel-space but this pipeline bins in data space pre-pixels — the attribute needs redefining, a design question not a port; (3) hex centres aren't a rectangular lattice, so raster's bin_step geometry-recovery convention breaks — hex size must be threaded separately; (4) hexbin is the canonical *unimplemented*-mark placeholder in ~12 tests across 4 crates — promoting it owns a swap dance the cheap three avoid entirely.

## Values

**Ride the substrate.** All three marks are recombinations of shipped machinery: DensityLowerer{TwoD} (+#42 determinism), the kde_2d grid, Band scales, Scale::Sequential + colorScheme (incl. the 0016 live-path threading), and raster's augment_scales pattern. Anything that forks rather than reuses that substrate is scope creep; the one sanctioned refactor is extracting the KDE-grid reconstruction into a shared helper, proven behaviour-identical by byte-identical density PNGs.

## Locked picks

- **Heatmap = the KDE-smoothed sibling of raster** (fixture semantics: `fill: density, bandwidth: 15`): Density2D's reconstructed kde_2d grid rendered as filled cells through the Fill Sequential ramp — every grid cell, not just occupied bins. Reuses DensityLowerer{TwoD}, `bandwidth` (silverman fallback), `colorScheme` (headless + live scheme threading like raster).
- **Cell v1 = pre-aggregated only**: categorical x × categorical y + numeric `fill:` column. Band on both axes (existing per-channel inference — nothing new); one rect per category pair via map_category/band_width; Fill Sequential built in augment_scales (numeric fill infers Linear otherwise — the recon's key trap). Domain anchoring: [0, max] when min ≥ 0, else [min, max]. A categorical (Utf8) `fill:` keeps the existing Colour-scale path (asserted, not built). Self-aggregating `fill: count/avg` needs a new CellLowerer — deferred with hexbin.
- **Contour = marching squares renderer-side**, over the same shared kde grid helper; N iso-levels; `stroke` is a literal colour v1 (fixture default steelblue); per-level ramp stroke deferred.
- **The `thresholds` collision is guarded, not papered over**: DensityLowerer reads `thresholds` as BIN COUNT (lower.rs:268-271); on a contour mark it means ISO-LEVEL COUNT (Mosaic semantics). Contour's lowerer registration must shield the density lowerer from the mark's `thresholds` attribute (thin wrapper filtering the attr view); a regression test pins that `thresholds: 12` on contour changes iso-line count, not SQL bin count.
- **Placeholder churn minimised**: hexbin/hexgrid stay Unimplemented (all ~12 negative tests untouched). Promoting `cell` orphans exactly ONE stub test (parse.rs dfspec_ac08 uses cell as the unimplemented-mark stub) — swap it to `voronoi` (outside this card and the follow-up). cellX/cellY stay Unimplemented.
- **Examples**: heatmap.yaml + contour.yaml reuse the raster two-blob cloud; cell.yaml gets a new small inline day×hour×value dataset. Fully headless track — gallery-eyeball gate, no app-window gating.

## Halt conditions

- Any pre-existing example PNG byte-diff → halt, revert. This now INCLUDES the raster trio (deterministic since #42) and specifically the density examples — they are the proof that the kde-helper extraction is behaviour-identical.
- Suite regression → halt before the next mark.

## Escalation triggers

- The kde-helper extraction cannot be made behaviour-identical (density PNG diffs) → surface the diff; do not "improve" the density rendering in passing.
- Cell's Band×Band rect geometry needs scale.rs changes (map_category/band_width are supposed to need zero) → surface before touching scale.rs.
- The contour attr-shield needs parser changes (it shouldn't — it's a lowerer-registration concern) → surface.

## Kill conditions

1. **Claim: heatmap/contour need zero new SQL.** Killed by the shared batch shape not sufficing (e.g. contour needing raw points rather than binned counts for acceptable quality) → pivot: contour drops to the follow-up with hexbin; heatmap/cell proceed.
2. **Claim: cell's pre-aggregated scope is a coherent v1.** Killed by no reasonable example working without aggregation → pivot: include the minimal CellLowerer (COUNT only) as a scoped extension, surfaced first.
3. **Claim: marching squares over the reconstructed grid yields clean iso-lines on the two-blob cloud.** Killed by visibly broken topology at v1 grid resolution → pivot: bump grid resolution for contour only, or defer contour.

## Verification posture

Everything code-side is `verifies: capability` (fully headless track: lowerer reuse, scale construction, marching-squares geometry, attr-shield, vocab/preflight, PNG byte-gates). The single stand-in: final appearance of the three new example PNGs — `verifies: stand-in (real thing is Hugh's gallery eyeball of the new PNGs), accepted because appearance quality is a human judgement` — backed by structural probes (scene path counts, encoded colour sampling per the #36 precedent).

## Budget

1.5 Claude working days, one PR off main (no dependence on the shell or interaction tracks). Tripwire day 2 → contour drops to the follow-up.

## Hot-wash

- surprised: the cost prior fully inverted — the "obvious next mark" (hexbin) is the worst v1 citizen on four independent axes; recon-before-spec keeps paying.
- recurred: the placeholder-mark swap dance (rect→hexbin last time) is now a known cost class of every mark promotion; the staging explicitly optimises around it.
- meta: ref-pinning the recon to origin/main (after the stale-premise incident) worked — zero errata this time.
