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

goal: "Live reactivity — param widgets re-execute downstream queries and cross-filtered selections propagate across views, turning the static first-render into an interactive dashboard"

cards:
  - 0005: "Reactive parameters with input widgets" (v2 — wire the param coordinator to live execution)
  - 0006: "Cross-filtered selections across linked views"

## Previously Shipped

- "First end-to-end render" — cards 0001 + 0004. Spec → SQL → DuckDB → GPU render → native GPUI window. Shipped at 7cb7005 (2026-04-29).

## Upcoming Sprint Candidates

- "Mark coverage breadth" — card 0008 (grammar of graphics mark library). Expand beyond dot/line/bar/density/regression to areaY, rect, text, rule, and the specialised marks (geo, hexbin, contour, raster).
- "Harden the render" — fold in the findings from `orbit/cards/memos/2026-04-29-first-render-followups.md`: literal channel values, vocab/runtime alignment, execution-conformance test layer, window/chart sizing.
