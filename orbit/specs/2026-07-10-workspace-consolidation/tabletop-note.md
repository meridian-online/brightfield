# Tabletop note — workspace consolidation: WorkspaceView prune + bottom dock

**Date:** 2026-07-10
**Cards in scope:** 0017 (authoring workspace); prune closes out 0016's superseded shell
**Output spec:** orbit/specs/2026-07-10-workspace-consolidation/spec.yaml

Authored by the driving agent (plugin 0.4.38 blocks model-invoked /orb:tabletop);
same hand-authored contract discipline as the 2026-07-10 crossfilter-render-fidelity
round. Sprint mandate: "Consolidate the workspace — prune the dead WorkspaceView …
add the missing bottom dock". The bottom dock unblocks Hugh's last open walkthrough
step (dock-drag-to-bottom, blocked 2026-07-10: no bottom drop target existed).

## Recon corrections (what the queue note got wrong)

The sprint queue recorded "their mod.rs:714 gates drop targets on dock existence".
Recon against the pinned gpui-component (b7e63cc2) corrects this:

- `has_dock` (mod.rs:711) gates the dock **toggle buttons** in TabPanel title bars,
  not drops. Drag-drops land only on an *existing* dock's TabPanel surfaces (tab
  bar, split zones) — `tab_panel.rs` `on_panel_drag_move`/`on_drop` only ever split
  or merge within reachable TabPanels.
- `DockArea::add_panel/move_panel(DockPlacement::Bottom)` auto-creates a bottom
  dock, but **nothing in their drag UI reaches it** — it is a programmatic API.
- Net: the only way Hugh's drag-to-bottom can work is to **seed the bottom dock**.
  Its collapsed form (their `Dock::render` special-cases bottom: a 29px title-bar
  strip stays visible when closed) is itself the drop/expand affordance.
- A dock's **last visible panel is never draggable** (`is_last_panel`), so a dock
  seeded with a permanent panel also prevents the trap where a panel moved into an
  otherwise-empty bottom dock becomes immediately un-draggable.
- The presentation escape hatch exists as public API: `remove_bottom_dock`
  (mod.rs:614), `DockArea::dump` → `DockAreaState.bottom_dock: Option<DockState>`,
  and public rebuild paths `DockState::to_dock` / `PanelState::to_item` with
  registered panel factories.

## Values

**The workspace consolidates without changing what ships.** Load-bearing:
presentation mode stays an honest consumer preview — a permanent 29px authoring
strip inside presentation would break the one promise that mode makes. Second
value: dead code goes away completely (WorkspaceView) without disturbing the live
pieces that happen to share its file (TogglePresentation, the `p` binding).

## Trade-offs

- **Seed panel = a real Log panel** (reload-feedback history), not an inert
  placeholder — acceptable scope: the feed already exists at the three
  `push_notification` call sites in shell.rs, and toasts vanishing is a known gap
  (#47 made errors sticky precisely because feedback got lost). The log is the
  toasts' persistent sibling. v1 content is reload/save feedback only.
- **Log panel closable=false in v1** — anchors the dock (an emptied bottom dock
  would linger as a dead strip; their TabPanel comment warns "visually empty and
  undroppable") and keeps moved-in panels draggable (never the last panel).
  Expensive-but-worth-it: a permanent tab is chrome, but it is the cheapest
  correct anchor. Revisit when a second bottom-dock citizen exists.
- **Bottom dock ships CLOSED by default** — the strip advertises the affordance
  without re-carving Hugh's current layout. Opening is one click (toggle button /
  drop). Acceptable.
- **Backfill instead of DOCK_STATE_VERSION bump** — restored layouts lacking a
  bottom dock get one appended post-load. A version bump would discard users'
  saved arrangements to add one dock; backfill is strictly kinder. Acceptable.
- **Presentation = remove + rebuild the bottom dock entity** — set_open(false)
  is not enough for bottom (the 29px strip survives it). Rebuild creates a new
  `Dock` entity, so the save-observer wiring must re-attach — sanctioned churn,
  named in the spec.

## Halt conditions

- **Byte-identity gate (exemption-free):** every one of the 27 example PNGs
  renders byte-identical to the `scratchpad/cfr-baselines` set (== main 5167a31,
  verified twice). All work is shell-side; the headless path must not move by a
  byte. Any diff = halt and diagnose, never exempt.
- **gpui-component stays unforked.** Everything must land through its public API
  at pin b7e63cc2. If the bottom-dock round trip (remove on presentation enter,
  faithful rebuild on exit) cannot be built on public API, halt and surface —
  do not fork, do not patch, do not ship the strip inside presentation.

## Escalation triggers

- `DockState`/`PanelState::to_item` turn out not to capture something the rebuild
  needs (e.g. open/closed state lives outside `DockState`) and no public
  supplement exists → surface with the exact missing field and the closest
  workaround (e.g. stashing `(DockState, was_open)` beside it) before widening
  scope.
- The prune turns out to have a live consumer of `WorkspaceView` I did not find
  (recon says: none — app imports only the action/context/bindings) → stop and
  re-scope rather than keeping a zombie export.

## Kill conditions

- **Claim: the bottom dock can round-trip presentation losslessly on public
  API.** Killed if moved-in panels cannot be restored faithfully → pivot: halt
  per above; the fallback of silently resetting bottom-dock contents on exit is
  NOT shippable (it loses user arrangement) — the round re-scopes to
  prune + seeded dock, with presentation handling escalated to Hugh with options.
- **Claim: the Log panel feed is a three-call-site tap.** Killed if reload
  feedback turns out to need new plumbing across the watcher boundary → pivot:
  Log panel ships with the dock seeded and wired to whatever subset of feedback
  is reachable at those sites; the rest is a named follow-up.

## Verification posture

- Prune, log model, visibility/backfill decisions, seeded/backfilled dock
  presence, presentation round-trip state: `verifies: capability` (unit +
  gpui-entity tests against the real WorkspaceRoot, as aws_ac03 established).
- Drag-to-bottom-dock, strip look, persistence across restart:
  `verifies: stand-in (real thing is the in-app gesture), accepted because`
  Hugh's walkthrough loop closes it same-week — it is literally his last open
  walkthrough step.

## Budget

1 Claude-day (prune is mechanical; dock work is one file cluster: shell.rs,
shell_model.rs, dock_state_file.rs, + new log model/panel). Tripwire: if the
presentation round-trip fights the entity/observer wiring past mid-round,
surface rather than burrow.
