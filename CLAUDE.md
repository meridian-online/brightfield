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

goal: "First end-to-end render — wire spec parsing, SQL emission, mark lowering, DuckDB execution, and GPU rendering so a Mosaic spec produces a visible, interactive output"

cards:
  - 0004: "Direct data source loading"
  - 0001: "Mosaic spec-driven visualisation (integration layer)"

## Upcoming Sprint Candidates

- "Runtime reactivity" — card 0005 v2 (param coordinator, live re-query) + card 0006 (cross-filtered selections). Static analysis layer shipped; next is wiring the subscriber graph to live execution.
- "Mark coverage breadth" — card 0008 (grammar of graphics mark library). Expand beyond dot/bar/line to the full Mosaic mark vocabulary.
