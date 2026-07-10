## Product Positioning

Brightfield targets analytics **authors** — the people who build dashboards and visualisations — much like Canva/Illustrator targets graphic designers. The primary user is an analyst who declares what they want to see in a Mosaic spec and gets an interactive, GPU-accelerated rendering.

Analytics **consumers** (people who view and interact with published dashboards) are a future audience. Good architectural choices now — portable specs via Mosaic, clean separation between authoring and rendering — ensure consumer-facing delivery can be enabled later without rearchitecting.

**Platform targets:** macOS, Linux, and Windows — matching Zed's GPUI platform support (Metal on macOS, Vulkan on Linux/Windows).

**Reference projects for data expression:** RillData (real-time analytics dashboards, DuckDB-native), MotherDuck (cloud DuckDB, query visualisation UX), apple/embedding-atlas (GPU-accelerated interactive data visualisation at scale).

## Workflow (orbit)

This project uses the orbit workflow: Card → Design → Spec → Implement → Review → Ship.

- `/orb:card` — capture a feature need with expected behaviours
- `/orb:distill` — extract capability cards from source material
- `/orb:discovery` — explore a vague idea through Socratic Q&A
- `/orb:design` — refine a feature card into technical decisions
- `/orb:spec` — crystallise interview into a structured specification
- `/orb:review-spec` — stress-test the spec before implementation
- `/orb:review-pr` — verify the PR against the spec's acceptance criteria

Artefacts live in `orbit/cards/`, `orbit/specs/`, and `orbit/decisions/`.

## Current Sprint

goal: "Finish the consolidation: implement the landed hexbin spec (hexbin + hexgrid + self-aggregating cell, orbit/specs/2026-07-10-hexbin-marks/); axis-inset round (edge-point trim — re-baselines every example PNG, needs a before/after gallery); keyboard-grammar discovery when Hugh is ready"

cards:
  - 0008: "Grammar of graphics mark library" (hexbin + hexgrid + self-aggregating cell)
  - 0017: "Authoring workspace" (next increment: sidebar profiling)

## Previously Shipped

- "First end-to-end render" — cards 0001 + 0004. Spec → SQL → DuckDB → GPU render → native GPUI window. Shipped at 7cb7005 (2026-04-29).
- "Live reactivity" — cards 0005 + 0006 + 0014. Param widgets drive queries (slider #25), cross-filtered brush + point selections propagate across views (#27–#31). Shipped 2026-07-04.
- "MIT-clean shipping floor" — LICENSE + clean-room GPL stub patch + cargo-deny CI gate (#34); Linebender stack bump to vello 0.9/wgpu 29 (#35). Shipped 2026-07-05.
- "Frame the canvas" — card 0016. WorkspaceView shell (title strip, presentation toggle via first GPUI action), window-hosted legends (swatch + gradient), live-path colorScheme. Shipped at 29fb46f (#38, 2026-07-05).
- Mark breadth instalments (card 0008, ongoing): density/regression (#21/#23), raster (#32), rect family (#28), sequential colour scale → true heatmaps (#36).
- "Accelerate to the authoring workspace" — three concurrent tracks, shipped 2026-07-06 at 1d1b9f4: legend click-to-filter (#41, card 0009), raster determinism (#42), the density mark family heatmap/cell/contour (#43, card 0008), and the DockArea authoring workspace with YAML editor + sidebar on gpui-component (#45, card 0017; supersedes #44).
- "Cross-filter render fidelity" — cards 0006/0008/0009/0017. Launch-anchored widen-only scales (gestures read as filtering: axes/colours hold still, query-widening data stays visible), legend selected-state (dim non-active swatches from the engine slot), live renderer-config seam (per-mark scheme/bandwidth/thresholds survive rebuilds via MarkInput.renderer_override). Shipped at 5167a31 (#50, 2026-07-10); in-app confirmed by Hugh (axes static, dimming visible).
- "Workspace consolidation" — card 0017. WorkspaceView pruned (the `p` action lives on in workspace_actions.rs); bottom Log dock (reload/save feedback history, seeded closed, old layouts backfilled); presentation removes/rebuilds the bottom dock (no 29px strip in consumer preview); stack-rooted dock items (at gpui-component pin b7e63cc2 the whole drag machinery gates on StackPanel parents — bare tabs are locked); "Dock at Bottom" menu-move bootstrap (a dock's only panel can never start a drag). Shipped at 00f617f (#52, 2026-07-11); in-app confirmed by Hugh (menu-move, bootstrap-then-drag, `p` round trip, log entry). Hexbin follow-up spec landed alongside (#51, planning-only — orbit/specs/2026-07-10-hexbin-marks/).

## Upcoming Sprint Candidates

- "Keyboard grammar" — VisiData-inspired command log + surfaces + scope-prefix keys (`orbit/cards/memos/2026-07-04-visidata-keyboard-grammar.md`). Needs `/orb:discovery`. The editor seam it rides now exists (#45).
- "Sidebar profiling" — card 0017's next increment: DuckDB column profiles (count/min/max/distinct/nulls) in the sidebar.
- "Continuous slider drag" — card 0015 (PR #26, planning-only). Needs tabletop: throttle vs debounce, query cancellation, async boundary.
- Remaining card 0008 marks: hexbin/hexgrid (follow-up spec staged; owns the placeholder swap dance), geo.
- Chores: RegressionLowerer stroke-group ORDER BY (jitter class from #42); upstream the gpui-component DockEvent two-liner; re-evaluate crates.io gpui when zed ships the post-split release; coordinator refresh on hot reload (watcher swaps scenes only — recorded in the 2026-07-10 crossfilter-render-fidelity tabletop note); legend multi-select toggle (Mosaic's shift-click union semantics; ours is single-select v1).
