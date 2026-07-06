# Tabletop — Authoring workspace v1: DockArea, spec editor, sidebar skeleton (card 0017)

**Date:** 2026-07-06
**Card:** orbit/cards/0017-authoring-workspace.yaml
**Output spec:** orbit/specs/2026-07-06-authoring-workspace-v1/spec.yaml
**Mode:** closed-space — Hugh greenlit the milestone (2026-07-05, "greenlight" + masonry watch); the design was fixed by two recons: the acceleration recon (gpui-component state, tag-route-dead) and the DockArea integration recon (their repo at b7e63cc2, file:line evidence, relayed 2026-07-06). Stage 1 (gpui pin → 1d217ee) already landed as #40.

**What good looks like** (author's seat): I open a spec and get a workspace — my chart in the middle, the YAML beside it, my sources on the left. I edit, hit save, the canvas re-renders. I drag panels where I want them and they're still there tomorrow. When a reload is rejected I see why in the app, not in a terminal. `p` still strips it all back to just the dashboard.

## Premise correction (recon errata)

The integration recon's headline ("Phase 0: bump the pin + migrate to gpui_platform") was computed against a stale working tree. Ground truth on main @ 7ae2b1f: the pin is already `1d217ee` everywhere (#40), `gpui_macos` still exists at that rev and our entrypoint compiles unchanged. Their `ui` crate depends only on `gpui` + `gpui_macros` — `gpui_platform` is their story-example entrypoint, not a library requirement. **We keep `Application::with_platform(gpui_macos::MacPlatform)` and add `.with_assets(...)`;** migrating the entrypoint is an escalation, not a plan.

## Values

**The workspace is chrome around an untouched core.** The vello path (`ChartElement`, chart/slider/legend elements, the PNG dump path) and the reload machinery (mtime watcher + `run_pipeline`) change zero lines. Everything new is hosting: panels wrap existing views, the editor writes to disk and lets the existing watcher do the rest, presentation mode flips panel visibility. Corollary (semantic-layer rule, standing constraint per Hugh's masonry watch): all new *logic* — save handler, presentation mapping, persistence round-trip, sidebar data derivation — lives in framework-free code; views stay shims.

## Locked picks

- **Dependency:** `gpui-component` as a git dep pinned at commit `b7e63cc2` (2026-07-03 main), `features = ["tree-sitter-yaml"]`, plus their assets crate (99 SVG icons, no fonts). No `gpui_web`/`reqwest_client`/`webview` — native `ui` pulls none of them (verified in their tree; the licensing gate re-checks the real lock).
- **Entrypoint:** keep `gpui_macos::MacPlatform`, add `.with_assets`. `gpui_component::init(cx)` at boot (globals only — theme/dock/input/root registries; no network, verified `lib.rs:107`). Root view wrapped in their `Root::new` (required for notification/dialog layers).
- **Panels v1:** center = `BrightfieldCanvas` wrapping today's `ChartView` (`closable(false)`, `panel_name` stable, NOT persisted — rebuilt fresh each boot from live pipeline state); right dock = YAML editor; left dock = sidebar skeleton.
- **Editor:** `InputState::code_editor("yaml").line_number(true)` seeded with the spec text; **save-driven, not change-driven** — `cmd-s` (new action, editor context) writes the buffer to the spec path; the existing 300ms mtime watcher does everything else. Zero reload-machinery changes. No LSP.
- **Reload feedback:** rejected reloads (parse error, chrome divergence "restart to apply") surface via `Root::push_notification` in addition to stderr. Card 0017 scenario 1's second clause, its own AC.
- **Sidebar v1:** data sources + their column names (derived headlessly from spec AST + the already-executed schema). No profiling stats — that's the card's later increment.
- **Presentation:** the gpui-free `PresentationMode` remains source of truth; ON = editor+sidebar `visible() == false` + docks collapsed, canvas full-bleed. **The 0016 "window never resizes on toggle" invariant is SUPERSEDED** — DockArea owns layout now; `framed_window_size` demotes to initial-window-size only, and the fww_ac01 oracle test is revised accordingly (recorded here, not by editing the shipped 0016 spec).
- **Keybinding:** bare `p` stays canvas-context-scoped. With an editor panel focused, `p` is text — accepted for v1 (VisiData-consistent: bare letters belong to the canvas). Focus-cycling keys and any `cmd-p` global alias go to the keyboard-grammar card. **Flagged to Hugh.**
- **Persistence:** `DockAreaState` (their serde type, `version` field) saved debounced on `DockEvent::LayoutChanged` + flushed on quit, to a Brightfield config-dir JSON; missing/corrupt/version-mismatch → default layout (their built-in reset flow). Dock state is shell-owned, never touches the spec.

## Halt conditions

- Any pre-existing example PNG byte-diff → halt, revert. (Structurally protected: the `BRIGHTFIELD_DUMP_PNG` path never constructs a DockArea — pinned by an AC.)
- Workspace test suite regression → halt before proceeding to the next AC.

## Escalation triggers

- Cargo refuses to unify the gpui graph (two-copies error) despite matching revs → surface the exact duplicate-crate output; do not chase feature flags blind.
- Anything forces the `gpui_platform::application()` entrypoint migration → surface with the forcing evidence; that's a scoped decision, not a silent swap.
- The markdown/html5ever/lsp-types/tree-sitter dependency cluster trips the cargo-deny licence gate → surface the offending crates; extending deny.toml's allow-list is Hugh-visible, not automatic.
- Brush/slider/legend-click interactions misbehave inside the center panel (event routing/hitboxes under the dock) → surface before workarounds.

## Kill conditions

1. **Claim: the dep unifies cleanly at matching revs.** Killed by irreconcilable graph duplication → pivot: pin gpui-component at the exact commit whose lock matches ours rev-for-rev; worst case vendor their ui crate (Apache-2.0).
2. **Claim: existing element interactions survive panel hosting unchanged.** Killed by dock-layer event interference → pivot: hybrid shell — canvas stays a plain div outside the dock; docks host only editor/sidebar; reassess DockArea-for-canvas at the next milestone.
3. **Claim: save-driven editing suffices.** Killed by the watcher fighting editor saves (mtime races, partial writes) → pivot: atomic write (temp+rename) first; if still racy, direct in-process reload call bypassing the watcher for editor saves only.

## Verification posture

- Dep/init/boot, panel hosting, dock drag/persist visuals, editor typing/highlighting, presentation hide: `verifies: stand-in (real thing is the workspace in the macOS app), accepted because the GPUI render tree cannot run headless` — each backed by the capability tests below.
- Save handler (buffer → atomic file write), presentation→visibility mapping, persistence round-trip (state serde + version + corrupt-fallback decision logic), sidebar source/column derivation, notification routing decision, PNG-path-never-builds-dock: `verifies: capability` (framework-free units per the semantic-layer rule).

## Budget

2 Claude working days, one PR (after the in-flight merge-train clears). Tripwire day 3 → sidebar skeleton drops to a follow-up PR.

## Hot-wash

- recurred: the semantic-layer rule did the scoping work again — every AC splits cleanly into a framework-free capability test + a stand-in eyeball.
- surprised: the recon's stale-premise incident — second time an agent read the checked-out branch instead of main. Future recon prompts must pin the ref explicitly (`git show main:...` or a detached worktree).
- friction: bare-letter grammar vs focused text editor is a genuine collision the VisiData memo predicted; v1 accepts canvas-scoped-only rather than pre-empting the grammar card.
