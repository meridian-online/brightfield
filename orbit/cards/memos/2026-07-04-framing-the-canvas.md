Research memo: how to frame Brightfield's visualisations — the workspace shell around the canvas, in the spirit of Zed's app layout. Covers the current shell state, Zed's workspace architecture, the gpui-component library (supersedes the 2026-04-28 placeholder memo), and framing patterns from Rill / MotherDuck / embedding-atlas / Observable / Tableau. Distilled from a four-track research pass on 2026-07-04; repo claims verified against `main` @ `e4379e4` (post the #27–#32 merge batch, which includes #30 legends/spacers hosting).

## The framing problem, precisely

Three layers get conflated under "framing":

**(a) Spec-level layout** — arranging views inside one dashboard. **Exists and works.** `brightfield-spec/src/layout.rs` resolves `hconcat`/`vconcat`/spacers/`legend:`/`input:` nodes to absolute pixel rects headlessly; `ChartView` absolutely positions each rect. Remaining gap: no reflow on window resize (`ChartState::set_dimensions` is dead code; the hot-reload watcher hard-rejects geometry changes — "restart to apply", `main.rs:610`).

**(b) Chart chrome** — axes, legends, titles around one plot. **Mostly exists headlessly after #30; the window lags.** Axes and the auto in-plot colour legend render in both PNG and window. Standalone `legend:` nodes are now hosted in the headless composite: `placed_legends`/`collect_legend_nodes` in `layout.rs`, `resolve_legends` in `main.rs` (content-sized via `colour_legend_size`, positioned via `render_colour_legend_at`), and the dashboard bounding box folds plot + slider + legend rects. `LegendChannel::Color` is `Implemented`; `ComponentKind::Legend` stays `Unimplemented` (`vocab.rs:255`) because **window hosting is the open follow-up** — the live GPUI window has no legend element, which is exactly the "window-gated" sprint item. Also open: legend click-to-filter (`as: $sel`), opacity/symbol legend channels, and the residuals in `2026-07-03-legends-spacers-followups.md` (legend-followed-by-sibling overlap, dropped-legend blank slot). Titles don't exist at all. `ChartLayout` margins (40/20/20/30) are hardcoded and duplicated across `brightfield-ui` and `brightfield-render`.

**(c) Workspace chrome** — docks, panels, tabs, editor around the whole authoring surface. **Absent.** The window root is a bare `ChartView` sized exactly to the dashboard bounding box, white background hardcoded (`chart_view.rs:122`), macOS titlebar as the only chrome.

"Legends/spacers hosting is window-gated" lives at layer (b); the Zed-inspired question is layer (c). They intersect at one point: the window's root view and size formula must change for either.

## Three options for layer (c)

**A — minimal hand-rolled shell (GPUI flex, no new deps).** Wrap `ChartView` in a workspace root view: title strip, padded content area, optional widget rail, plain `div()`s on the existing gpui pin (`rev = "14f0a254"`). Confirmed cheap: `ChartView` has a plain constructor and nothing but `.size_full()` assumes window-rootness; chart pixels flow through a cached raster, so chrome quads never touch the Vello path; element origins come from live paint bounds, so translation inside a padded area is safe. Effort: days — #30 already paid the layout-side debt (placed_legends, bbox fold); what's left is the shell divs plus the window legend element. No gpui lock, licensing clean, but no docks/tabs/editor/persistence.

**B — adopt gpui-component's dock/panel system** (github.com/longbridge/gpui-component). `DockArea`/`Dock`/`TabPanel`/`Tiles`, `PanelRegistry`, serde layout persistence; Apache-2.0; production-hardened in Longbridge Pro. A chart hosts behind their small `Panel` trait — the dock never touches the paint path, so the vello `ChartElement` is opaque to it. Their `input/` crate is a real code editor (tree-sitter highlighting) — matters the day we want an in-app spec editor. **The sharp edge is the gpui pin:** their git main pins zed rev `1d217ee` (2026-06-12) vs our `14f0a254` (2026-04-22); cargo treats different revs as different crates (their issue #2532 is exactly this failure), and crates.io `gpui-component` trails main by months. Adopting B means surrendering the pin to their cadence.

**C — clean-room Zed-style workspace.** Reimplement `Workspace { center: PaneGroup, docks, status_bar }` with the `Panel`/`Item` trait pattern. Zed's `workspace` and `ui` crates are **GPL-3.0-or-later** (gpui itself is Apache-2.0) — concepts only, no copying. A few thousand lines before it's useful; months of shell work displacing chart work.

## Recommendation

**Stage A now; decide B-vs-C only when the shell needs a second region; shape A so B can absorb it.** C is ruled out on effort — Brightfield's differentiation is the Mosaic-to-GPU pipeline, not drag-to-redock tabs. B is premature: its one material cost (pin sovereignty) buys docks and an editor pane nothing in the current sprint needs.

First milestone — **"the framed window"**:
1. New `WorkspaceView` in `brightfield-ui` as window root, mounting the existing `ChartView` in a padded content area; dashboard title (spec filename or new `title:` attribute) in a thin header strip.
2. **Window legend hosting:** a `LegendElement` sibling in `ChartView` at the `placed_legends` rect, painting what `render_colour_legend_at`/`colour_legend_size` already produce for the composite (#30 refactored them into positioned APIs precisely so an element can reuse them). Promote `ComponentKind::Legend` out of `Unimplemented` once live. Closes the window-gated item. Mind the `ChartElement`/`SliderElement` index coupling with the coordinator when adding siblings.
3. Window size formula = dashboard bbox (which already folds legend + slider rects) + chrome extents.
4. **Presentation toggle** — one action hiding header/rail, leaving canvas + spec-declared widgets. Cheap now; enforces the author/consumer seam from day one.

Sprint ordering: **Scale::Sequential → framed window → legend click-to-filter.** Sequential is `brightfield-render` work with no shell dependency and feeds the legend swatches the milestone hosts; click-to-filter needs a hosted, hit-testable legend plus the existing `CrossfilterCoordinator` dispatch — its other prerequisite, typed categorical predicates, already shipped in #31.

The B trigger: the day the roadmap commits to an in-app spec editor or data sidebar, adopt gpui-component rather than hand-rolling — and pay the pin migration then, when a gpui bump is on the table anyway. Keep `WorkspaceView` thin so a `DockArea` can replace it without touching `ChartView` or below.

## Spec ↔ shell rule

**Anything a consumer needs ships inside the spec-derived canvas; the shell owns only authoring state.** Portability test for any new feature: headless PNG output and presentation mode must both be derivable from the spec alone.

- In-canvas (spec-owned): `hconcat`/`vconcat`/spacers, plot sizes, `input:` widgets at their declared positions (resist moving them into a shell-owned filter bar — that forks spec from render), `legend:` components via `placed_legends` (hosting legends in chrome would strip them from PNG export and future published dashboards), per-plot titles, future cross-filter side panels (embedding-atlas pattern: "sidebar" = more Mosaic clients on the same selection graph — nearly free for us, strongest expression of the positioning).
- Shell-only (never serialized into YAML): dock/panel visibility, editor placement, panel sizes, theme, presentation mode, layout persistence; data sidebar / column explorer (fed by DuckDB off the spec's sources — MotherDuck's Column Explorer is the benchmark).

## Framing capabilities, prioritized (survey distillation)

1. Spec editor + live canvas side-by-side, re-render on edit (table stakes — but the hot-reload watcher already gives the Observable-Framework variant: external editor + live preview).
2. Data sidebar with column profiling (table stakes; DuckDB computes it).
3. Param/filter widgets from the spec in a consistent place (table stakes; substrate shipped in cards 0005/0006).
4. Multi-view grid with per-view cards — title, legend slot, local params (table stakes; Tableau's lesson: legends are cards inside the composition, they publish with the dashboard).
5. Presentation/consumer mode toggle (table stakes, cheap, high leverage).
6. Chrome-as-coordinated-views — cross-filtering side panels (differentiator; card 0006 is the substrate).
7. Inspector with visual ↔ spec round-trip (differentiator; defer — Rill 0.50's lesson: visual editing must be a projection of the spec, never a parallel model).

## Licensing flags

- gpui: Apache-2.0. Zed `workspace`/`ui` crates: GPL-3.0-or-later (Zed has said they won't relicense) — concepts only.
- gpui-component: Apache-2.0.
- **Audit owed regardless of framing choice:** zed repo issue #55470 reports that building gpui from the zed git tree (as we do) pulls `sum_tree → ztracing → zlog`, which are GPL. Whether that matters depends on Brightfield's own license posture.

## Same-day addendum: decisions + licensing/cadence verification

Hugh answered the open questions: **(1) Brightfield will be MIT licensed. (2) Leaning toward an in-app spec editor with keyboard ergonomics inspired by VisiData** (see `2026-07-04-visidata-keyboard-grammar.md`). **(3) No position on gpui rev cadence** — resolved below by verification.

**The GPL chain is real, verified locally, and small.** Our `Cargo.lock` pulls exactly three GPL-3.0-or-later crates from the zed tree: `ztracing`, `ztracing_macro`, `zlog` (every other zed-tree crate we pull is Apache-2.0; `derive_refineable`/`gpui_shared_string`/`gpui_util` lack a license line at our rev — minor audit items). The entire chain enters through **`sum_tree` alone** (`ztracing.workspace = true`, non-optional; sum_tree's source uses only `ztracing::instrument`, two use-sites). `ztracing` itself is ~95 lines of no-op macros and re-exports of the MIT-licensed `tracing` crate; `zlog` ~1.7k lines. **Fix: clean-room no-op stubs for the three crates via `[patch."https://github.com/zed-industries/zed"]`** — hours of work, keeps the current pin, makes the binary MIT-shippable, survives pin bumps (the compiler flags any new API surface). Add a `cargo-deny` license gate to CI so regressions can't sneak in.

**The crates.io escape hatch is closed today** (this kills the tidy "just ride published releases" answer). crates.io `gpui` is 0.2.2, published 2025-10-22 — a **pre-platform-split snapshot**: `gpui_macos` is not on crates.io at all (404; not even `publish`-enabled in-tree), so our dependency shape can't be expressed against it. Zero gpui releases in ~8.5 months, publish is a hand-run script with no stated cadence, and 0.2.2 has an unresolved macOS-26 Metal crash report (zed #46486). Consolation: the published tree IS GPL-free (the `gpui_*` republished deps are all Apache-2.0), and zed's README already documents a post-split `gpui`/`gpui_platform` release that hasn't shipped — one is clearly intended, date unknown.

**gpui-component, verified:** crates.io `0.5.1` (2026-02-05) contains the dock system, the tree-sitter editor (`input` + `highlighter`), and charts — and it pairs with crates.io `gpui ^0.2.2` as a single unified crate (no type-duplication; their #2532 failure is git-path-only). But main is ~440 commits ahead, their publishing is frozen on zed's (they only release when zed does), and their docs recommend git. So published-B exists but is Oct-2025-vintage underneath.

**Cadence resolution — keep pin sovereignty; the stub patch is what buys it.** With the GPL chain stubbed, staying on zed git revs of our choosing is MIT-clean, so there is no licensing pressure to ride anyone's cadence. When the editor milestone arrives (B trigger), adopt gpui-component **at a tagged release, pinning our gpui to the zed rev that tag blesses** — we inherit their rev *choice* at each adoption point but fully control *when* to bump. Re-evaluate the all-crates.io stack if/when zed ships the post-split gpui release; that release, not 0.2.2, matches the API our code is written against.

## Remaining open questions

1. When to do the ztracing/zlog stub patch — it's owed before the first distributed binary; doing it early (it's small) removes the constraint from all later decisions.
2. Whether to spike published `gpui 0.2.2` + `gpui-component 0.5.1` on current macOS (the #46486 crash report) — only worth it if we want the editor sooner than zed's next publish.

→ Card for "the framed window" milestone (window legend hosting + layer-c minimal shell + presentation toggle) when framing enters a sprint; Scale::Sequential should precede or accompany it.
