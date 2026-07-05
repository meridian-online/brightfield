# Tabletop — Workspace shell: the framed window

**Date:** 2026-07-05
**Cards in scope:** 0016 (workspace shell — the framed window)
**Output spec:** orbit/specs/2026-07-05-framed-window/spec.yaml (to be crystallised by /orb:spec)

**Capability ambition:** the app window becomes a faithful frame for the spec-derived canvas — everything a consumer would see lives in the window exactly as it ships in the PNG, wrapped in authoring chrome that can get out of the way.

**Goal (Q1, locked):** put a frame around the canvas — title strip, real legends in the window, a key to hide the chrome — without changing a single pixel of what the PNG renders.

**Carve:** one card, one spec. The four pieces (shell root, title, window legends, presentation toggle) all change the same two things — the window root view and the size formula — so carving would create a dependency chain, not remove one.

---

## Values

**Load-bearing: window/PNG parity.** The live window finally shows everything the headless composite shows, which makes every future eyeball gate trustworthy end-to-end. Parity pulled two things into scope during the session: live-path `colorScheme` threading (deferred from #36 — without it a `colorScheme: blues` raster renders viridis cells *and* a viridis hosted legend in the window while its own PNG renders blues), and the byte-identity gate on all pre-existing PNGs.

Constraints, not drivers: the **author/consumer seam** (presentation mode must show exactly the spec-derived canvas — derivable from the spec alone) and **thin-shell replaceability** (WorkspaceView stays a shim a gpui-component DockArea can absorb at the editor milestone; semantic-layer rule applies — registry-as-data, state machines headless, views as translation shims).

## Locked picks

- **Presentation toggle key: bare letter `p`, canvas-scoped.** First GPUI action in the product; the action name is the durable artifact, the binding is remappable config. Commits to the VisiData bare-letter grammar now — a future editor pane gets its own key context.
- **Toggle sizing: window keeps its size; canvas re-centres** in the freed space (existing centring behaviour). No window-resize animation, symmetric restore.
- **Title: `meta.title`, filename fallback — shown in both** the native macOS titlebar (`set_title`, document-app convention) and the shell header strip (the styled, presentation-aware surface). No parser change: `meta.title` already exists in the AST (typed `Meta` block, Mosaic-aligned, previously unconsumed).
- **Legends ship display-only**; hit-testing arrives with click-to-filter (next card). A display-only element needs no coordinator index — the ChartElement/SliderElement index coupling only becomes real once dispatch exists.
- **colorScheme live-path threading folded in** (`MarkInput`/`LivePlotMeta` scheme field), closing #36's deferred parity gap.

## Trade-offs

- Hand-rolled shell instead of gpui-component now — **acceptable**: DockArea absorbs a thin WorkspaceView later; docks/tabs/persistence deliberately not built.
- Display-only legends — **acceptable**: click-to-filter adds hit-testing against the same rects next card.
- Bare `p` before the keyboard-grammar discovery — **acceptable**: worst case is a one-line keymap remap; the action name survives.
- colorScheme threading through the live path — **expensive-but-worth-it**: touches `crossfilter.rs`, freshly churned by #36, but the marquee demo (blues heatmap + hosted gradient legend) must match its own PNG.
- Still no resize reflow — **acceptable**: chrome does not resurrect `set_dimensions`; window resize keeps showing margin as today.
- PNG byte-identity — **halt-trigger**, not a trade-off (see below).

## Halt conditions

- **Any pre-existing example PNG byte-diffs** during shell/legend/toggle work → halt, `git revert` the offending commit, re-establish byte-identity before proceeding. (New examples added by this spec are exempt.)
- **LegendElement cannot reproduce composite legend output without forking `legend.rs` draw code** → halt; this kills the reuse claim (kill condition 1) — take the rasterise pivot rather than fork the seam.

## Escalation triggers

- Legend-reuse divergence exceeds ~50 LOC, or any gpui type creeps toward `brightfield-render` → surface the divergence diff, propose the rasterise-composite-region pivot.
- GPUI action registration demands app-level keymap restructuring beyond one action + one key context → surface, propose a raw key-down handler for v1 and defer the action registry to the keyboard-grammar card.
- colorScheme threading turns out to touch SQL / plan_hash / query cache (it should be render-only) → surface, re-defer to its own follow-up.
- Window + chrome extents exceed the primary screen for any example → surface with measured sizes, propose a clamp policy.
- Budget tripwire: work enters day 3 → surface scope, propose dropping to the legends-first cut.

## Kill conditions

1. **Claim: the positioned legend APIs (`render_colour_legend_at` / `render_sequential_legend_at` + size fns) are reusable verbatim by a window element.** Killed by irreconcilable font/DPI/text divergence → pivot: rasterise the composite's legend region into the element — bitmap parity by construction, placement logic kept.
2. **Claim: chrome never touches the composite path.** Killed by a PNG byte-diff traced to shell work → pivot: revert shell, re-scope to the legend-only increment (lateral held in reserve).
3. **Claim: one GPUI action + a canvas key context suffices.** Killed by framework-level keymap buy-in → pivot: raw key-down handler now; action registry deferred to the keyboard-grammar card.
4. **Claim: scheme threading is render-only.** Killed by SQL/plan-hash/cache involvement → pivot: re-defer, document the live-path gap as before.

Laterals held in reserve (not picked): gpui-component DockArea (adopt at the editor milestone, at a tag); legend-only increment without shell (fallback); rasterise-composite-legend-region (pivot for kill 1). Rejected permanently: chrome-owned legends (legends publish with the dashboard — spec↔shell rule); menu item instead of a key (keyboard-first bet).

## Verification posture

- Framed canvas: `verifies: stand-in (real thing is a human seeing the framed macOS window), accepted because the GPUI render tree cannot run headless` — backed by a capability test of the chrome-extent math.
- Dashboard title: `verifies: capability` for resolution logic (meta.title → filename fallback, headless unit); `verifies: stand-in (display in titlebar/header), accepted because window pixels need the app` for the visible strip.
- Window legends: `verifies: stand-in (eyeball screenshot vs PNG), accepted because window pixels need the app` — backed by capability tests of placement/size (existing `resolve_legends` coverage) and of live-path scheme stops (headless probe).
- Window fits the frame: `verifies: capability` — the size formula (bbox + chrome extents) is a pure function.
- Presentation toggle: `verifies: capability` for the presentation state machine (plain state, headless, per the semantic-layer rule) and for spec-derived rects being unchanged by shell state; `verifies: stand-in (chrome visibly hides), accepted because window pixels need the app`.

## Budget

**2 working days at Claude-execution pace** (matches the #36 shape: similar scope, adversarial review included), one PR. Escalation tripwire at day 3 → drop to legends-first.

## Implementation notes (Q8 — adjacent code)

- `brightfield-ui`: `chart_view.rs` donates white-bg/centring upward to a new `workspace_view.rs`; new `legend_element.rs` mirrors `chart_element.rs` (vello scene → RGBA → `RenderImage` choke point, one `paint_image`); `slider_element.rs` untouched.
- `brightfield-app`: `main.rs` — window-size formula (~line 868) gains chrome extents; `resolve_legends` (line 166) reused for live placements; `meta.title` consumption + `set_title`; action/key-context registration. `crossfilter.rs` — scheme field through `MarkInput`/`LivePlotMeta`.
- `brightfield-spec`: no changes — `meta.title` already parses (`ast.rs:49`).
- `brightfield-render`: `legend.rs` strictly read-only (untouched-seam constraint); `vocab.rs` promotes `ComponentKind::Legend` out of Unimplemented.

## Hot-wash

- **recurred:** parity decided everything — once picked as the load-bearing value it settled scheme threading, the PNG gate, the verification posture, and the halt conditions with no further debate.
- **surprised:** `meta.title` was already in the AST, typed and Mosaic-aligned — the card's "new `title:` attribute" was free. The ChartElement/SliderElement index-coupling worry also dissolved on inspection (separate index spaces; display-only legends need neither index nor coordinator).
- **friction:** plugin/repo convention split — the orbit CLI and skill templates expect `.orbit/`, this repo uses `orbit/`; `/orb:design` was renamed to `/orb:tabletop` and is user-invocable only; the STYLE.md @-include resolves to nothing here.
- **meta-patterns-for-future-tabletops:** recon-before-tabletop (verifying seams on current main) converted two design questions into facts before Q1 — cheap and high-yield. Deferred items from the last merged PR resurface as debts against the new value (parity vs #36's scheme deferral) — read the previous spec's `deferred:` list at every tabletop open.
