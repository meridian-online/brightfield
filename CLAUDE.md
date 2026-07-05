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

goal: "Accelerate to the authoring workspace — three concurrent tracks: (1) shell: gpui pin bump then gpui-component adoption (DockArea + spec editor + sidebar, card 0017); (2) interaction: legend click-to-filter (card 0009); (3) marks: raster determinism chore then hexbin/contour/heatmap (card 0008)"

cards:
  - 0017: "Authoring workspace — docked panels, spec editor, data sidebar"
  - 0009: "Multi-view dashboard composition" (legend click-to-filter scenario)
  - 0008: "Grammar of graphics mark library" (hexbin, contour, heatmap/cell)

## Previously Shipped

- "First end-to-end render" — cards 0001 + 0004. Spec → SQL → DuckDB → GPU render → native GPUI window. Shipped at 7cb7005 (2026-04-29).
- "Live reactivity" — cards 0005 + 0006 + 0014. Param widgets drive queries (slider #25), cross-filtered brush + point selections propagate across views (#27–#31). Shipped 2026-07-04.
- "MIT-clean shipping floor" — LICENSE + clean-room GPL stub patch + cargo-deny CI gate (#34); Linebender stack bump to vello 0.9/wgpu 29 (#35). Shipped 2026-07-05.
- "Frame the canvas" — card 0016. WorkspaceView shell (title strip, presentation toggle via first GPUI action), window-hosted legends (swatch + gradient), live-path colorScheme. Shipped at 29fb46f (#38, 2026-07-05).
- Mark breadth instalments (card 0008, ongoing): density/regression (#21/#23), raster (#32), rect family (#28), sequential colour scale → true heatmaps (#36).

## Upcoming Sprint Candidates

- "Legend click-to-filter" — card 0009's interactor scenario: `legend: color as: $sel` filters linked views. Prereqs shipped: hosted legend (card 0016), typed categorical predicates (#31), CrossfilterCoordinator dispatch.
- "Continuous slider drag" — card 0015 (PR #26, planning-only). Needs `/orb:design`: throttle vs debounce, query cancellation, async boundary.
- "Keyboard grammar" — VisiData-inspired command log + surfaces + scope-prefix keys (`orbit/cards/memos/2026-07-04-visidata-keyboard-grammar.md`). Needs `/orb:discovery`.
- Remaining card 0008 marks: hexbin, contour, heatmap/cell (sequential scale now available), geo.
