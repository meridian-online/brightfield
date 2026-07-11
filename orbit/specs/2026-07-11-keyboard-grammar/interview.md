# Discovery: Keyboard Grammar (VisiData-inspired)

**Date:** 2026-07-11
**Interviewer:** Claude (Opus 4.8, ultracode)
**Card:** none yet — candidate card 0018 ("keyboard grammar + surfaces"); seed memo `orbit/cards/memos/2026-07-04-visidata-keyboard-grammar.md`
**Mode:** discovery
**Companions:** `keymap-research.md` (scored candidate digest), `keymap-candidate.html` (cheat-sheet artifact), `keymap-research-raw.json` (full 12-agent output)

---

## Context

Brightfield wants a VisiData-inspired keyboard-first authoring UX: single-key verbs, a scope-prefix
system, a command palette, and (eventually) a command log that turns every session into a
replayable/undoable program. The seed memo ranks five priorities: (1) command-log spec-edit stream,
(2) SQL/stat surfaces, (3) scope-prefix grammar + palette, (4) columns-as-lenses, (5) never-block async.

This session was grounded by three parallel research workflows before any question was put to Hugh:
- **Brightfield sweep** (4 maps): prior art across 40+ specs, the #45 editor seam, the command-log
  substrate, the keyboard/action/focus infra.
- **VisiData source study** (3 facets) against the local checkout at `~/github/saulpw/visidata`:
  grammar/dispatch, cmdlog/undo/macros, sheets + columns-as-lenses.
- **Keymap ergonomics research** (12 agents): reference models (VisiData, vim, Helix/Kakoune, Zed,
  Linear/Superhuman/Gmail, KLM/GOMS) + a Brightfield command-frequency model → a scored candidate
  keymap → 3× adversarial critique → reconcile.

**Grounding headline — what's built vs greenfield.** The dispatch *plumbing* exists: GPUI
actions + remappable keymap-as-data + named key-contexts (bare `p`, `cmd-s`), every action carries a
global longname, and the analysis layer already addresses every node by `ComponentPath`
(`root/vconcat[0]/plot[1]/mark[dot]`) with a `parent_plot()` helper. Genuinely greenfield: the
scope grammar itself, the palette, the surface/stack model, the command log, **and a sub-panel focus
model** — canvas focus is claimed only by `on_mouse_down` (`shell.rs:242`) + a one-time launch focus
(`main.rs:1527`), so there is *no keyboard route from the editor to a chart today*.

---

## Q&A

### Q1: First-card spine
**Q:** The memo lists five priorities; the first card can't be all of them. Where does its spine sit?
**A:** **Key-grammar + palette** (memo priority #3) — the visible, learnable increment that rides the
most existing substrate (GPUI dispatch + `ComponentPath` + native longnames). The command-log spine
(#1) is deferred to a later card.

### Q2: Prefix semantics — Hugh reframes the question
**Q (asked):** Is the g/z prefix axis *containment* (re-target a node up/down the tree) or *magnitude*
(modify the same target)?
**A:** Hugh declined the binary: *"I'm not in the best position to pick individual keys to commands. I
want them to be intuitive and ergonomic. Is there some research we should run or a reference we can
use?"* → This reframed the whole card: **the keymap is not something Hugh authors by taste; it is
something the design derives and defends.** A new acceptance criterion falls out — *every binding is
justified by an explicit frequency/mnemonic/convention score, not chosen by hand.*

### Q3: Ergonomic objective (what the scoring optimizes for)
**Q:** When the research scores candidate keymaps, what should "ergonomic" optimize for?
**A:** **Balance frequency + mnemonic + convention, and show where they conflict** — so Hugh reacts to
a scored proposal, not a blank keymap. (Drove the 12-agent research; a 4th motor-cost lens was folded
into every score's conflicts per the HCI study.)

### Q4: Scope axis (resolved by the research, ratified by Hugh's downstream choices)
**Finding:** The research converged on **selection-first**, dissolving the containment-vs-magnitude
binary. Scope = *where focus sits* on the `ComponentPath` tree (Helix/ranger-style tree-nav: `h/l`
depth, `j/k` siblings); a bare argumentless verb acts on the focused node; the **only** surviving
prefix is `g` = broadcast a runtime verb to the whole dashboard (always resolves to root, so it can
never mean "delete my parent"). `z` is **dropped** (Enter-dive gives containment-down more cheaply),
which frees `z` for future zoom-extent and kills the `z`=zoom collision.

### Q5: v1 ceiling
**Q:** How far does v1 reach, given two deferred builds bound it — the command log (gates every
spec-edit verb + undo) and the focus model (now a mandatory prerequisite)?
**A:** **Nav + palette + focus model + built runtime verbs, PLUS `c` = set-colour-scheme as a
*transient* live-preview** via the existing `renderer_override` seam (no log needed to preview;
persisting still waits for the log). The full spec-edit family + undo are **reserved-and-visible** in
the palette (greyed, unbound) so the vocabulary is taught before the keys exist.

### Q6: The `g` convention
**Q:** Does `g` mean "broadcast to dashboard" (diverging from the strong `g` = "go to / jump"
convention in Gmail/Linear/vim)?
**A:** **Accept `g` = dashboard-broadcast** (runtime-only, always root), taught by a visible breadcrumb
showing the resolved target + a which-key overlay. Divergence surfaced, not hidden.

### Q7: Channel altitude
**Q:** Does keyboard focus descend to the channel (x/y/color) in v1, or stop at the mark?
**A:** **Floor at mark.** v1 focus goes dashboard → view → mark and stops; channel-level edits live in
the palette (by longname) until a later card. No mark→channel sub-focus target is built this card.

### Q8: Acceptance bar
**Q:** How do we know the keyboard grammar "works"?
**A:** **Mouse-free authoring path + a keyboard-only walkthrough demo.** From a cold launch, every
navigation, verb, and palette action is reachable by keyboard alone — you never *need* the mouse to
author or navigate — and the sign-off artifact is a recorded keyboard-only screencast (mirroring
Hugh's in-app eyeball-verification loop, doubling as the living demo). Pointer gestures
(brush/cross-filter) remain as data interactions flowing through params — they are not authoring.

---

## Summary

### Goal
Ship the first, self-contained increment of a VisiData-grade keyboard-first authoring experience: a
**selection-first** grammar where scope is focus on the `ComponentPath` tree, a bare verb acts on the
focused node, a single `g` prefix broadcasts runtime verbs to the whole dashboard, and a fuzzy
**Space palette** carries discoverability so single keys stay terse. The keymap is **derived and
scored**, not hand-authored. The felt outcome: keyboard-first authoring you can prove mouse-free.

### Constraints
- **Framework-free semantic layer** (standing rule): command registry, focus state machine, scope
  resolver, and palette filter live as gpui-free data/state machines; GPUI `actions!`/keymap only
  *adapt* them. Views stay shims. (rust-ui-field-scan five-part rule.)
- **Rides GPUI dispatch** — `actions!` + remappable keymap-as-data + named key-contexts; every action
  carries a global longname (the palette corpus is nearly free). Two contexts exist
  (`BrightfieldWorkspace`, `BrightfieldEditor`); a third for mark scope would be added similarly.
- **Focus model is a mandatory greenfield build** — canvas focus is mouse-only today; a global
  (context=None) editor↔canvas focus toggle + per-node focus targets (rendered as *descendants* of the
  single workspace context) are prerequisites, or the grammar is unreachable without a click.
- **PNG-byte gate holds** — new surfaces/overlays stay chrome-only; headless PNG output unchanged.
- **Presentation mode** — every new surface/overlay must declare its presentation-mode visibility.
- **macOS-eyeball verification** for the GPUI wiring (per project convention); the framework-free
  semantic layer is headless-unit-testable.
- **Scope bounded by two deferred builds** — the command log (all spec-edit verbs + undo) and the
  mark→channel focus target (channel-level bare verbs) are explicitly out of this card.

### Success Criteria
- From a cold launch, an author performs a full build/inspect loop **without the mouse**: `cmd-Enter`
  into the canvas → `h/l/j/k` walk to a target mark → bare `f` filter / `g f` cross-filter-all → `t`
  select / `Esc` clear → `Space` palette finds any verb by meaning and shows its bound key inline →
  `c` live-previews a colour scheme → `p` presentation → `cmd-s` save.
- Every reserved spec-edit verb (`m a e d`, `undo`) appears in the palette by longname, greyed with a
  "needs command log" hint — vocabulary taught before the keys exist.
- Every bound key is justified by a recorded frequency/mnemonic/convention score (+ motor-cost note).
- A pending prefix (`g …`) shows a which-key overlay; `Esc` cancels it instantly
  (`clear_pending_keystrokes`), and no bare verb dispatches while the palette is open (focus-invariant
  test).
- Sign-off: a recorded keyboard-only walkthrough screencast.

### Decisions Surfaced
- **Spine = key-grammar + palette** (memo #3), command-log deferred. Chosen over the command-log spine
  (#1) for visibility/buildability. → candidate MADR.
- **Keymap is derived + scored, not hand-authored** — balance frequency/mnemonic/convention + motor
  cost, conflicts shown. (Hugh's explicit reframe.) → candidate MADR.
- **Selection-first scope model** — focus IS scope on `ComponentPath`; bare verb acts on focus; `g` =
  dashboard-broadcast (runtime-only, always root); `z` dropped. Chosen over containment-prefix and
  magnitude-prefix; grounded in Helix (tree-walk selection) + Zed (context-tree position) + VisiData's
  own `s`/`gs` impurity. → **primary MADR** (the load-bearing architectural decision).
- **`g` = dashboard-broadcast** accepted over `g` = "go to", carried by breadcrumb + which-key.
- **v1 ceiling** = nav + palette + focus model + built runtime verbs + `c` colour-scheme live-preview;
  spec-edit family + undo reserved-and-visible; channel altitude floored at mark.
- **Nav keys** = ranger/miller tree convention (`h` out, `l` in, `j/k` siblings; `Enter`/`q` VisiData
  alternates), rejecting hjkl screen-direction (the tree has no honest screen projection).
- **`Esc` precedence ladder** = dismiss overlay › cancel pending prefix › clear selection; does NOT
  auto-pop (protects the shipped Esc-clears memo; pop lives on `h/q`).
- **`cmd-r` reload demoted** off bare `r`, gated on a dirty-buffer check (bare `r` re-parsing disk
  could silently discard unsaved editor edits).

### Implementation Notes (means-level, for the spec author)
- **Two registries, keystroke→longname decoupled; keymap as data.** Never branch on keys inside a
  handler; scope/prefix variants are extra bindings onto the same named action. (VisiData
  `settings.py:359` addCommand vs `:394` bindkey.)
- **The palette is nearly free**: every GPUI action already has a global longname and the keymap is
  data, so the fuzzy corpus + inline keystrokes are queryable today. Palette must be focus-scoped by
  the same `scope_applicability` predicate the bare keys use (palette and single-key agree at every
  altitude), fuzzy over longname+help, frequency-ranked, and anchored **outside** the WORKSPACE
  subtree (the canvas doesn't stop propagation, `shell.rs:230`) or bare verbs leak underneath it.
- **`cmd-shift-p` (palette) and the focus toggle must bind `context=None` on `WorkspaceRoot`** — the
  shared ancestor where dispatch bubbles (`shell.rs:70`). WORKSPACE and EDITOR are *sibling* panels;
  a WORKSPACE-only palette binding is unreachable from the editor and strands the spine. `cmd-s` is
  currently editor-scoped (`shell.rs:90`) — re-bind global for save-from-anywhere (audit native-menu
  accelerator clash across macOS/Linux/Windows).
- **Focus targets render as descendants of the single WORKSPACE key-context div** (move an inner
  focus ring; don't give each node its own context) or bare verbs go dead when focus lands on a mark.
- **Which-key uses GPUI's existing ~1s pending engine** (type-through, timeout-gated); `Esc` calls
  `window.clear_pending_keystrokes` so `g`-then-`Esc` aborts instantly.
- **`c` = set-colour-scheme rides the existing `renderer_override` / `MarkInput` seam** for transient
  preview — needs an honest "previewing vs saved" affordance.
- **The command log, when it lands, is the natural next card**: a `SpecEdit` enum in brightfield-spec
  (`apply(&mut Spec)` — the missing AST mutation API), reserialised via the existing `impl Serialize
  for Spec`, appended to a `FeedbackLog`-shaped log; both the editor `cmd-s` path and the coordinator
  `commit_*`/`propagate_*` seams re-expressed as `SpecEdit` producers. VisiData warns: target by
  **stable spec-node identity** (never positional row/col), use a **typed** edit (never execstr eval),
  keep the log **append-only with an undo cursor** (VisiData's undo deletes the row — a category
  error here), and log the **semantic result** of a gesture (param=[a,b]) not raw pixels, coalescing
  a drag into one entry. AST→YAML reserialise is idempotent but **not** text-preserving (no source
  spans on nodes) — the text-fidelity fork is the log card's to resolve.

### Open Questions (intent-level, for the spec / later cards)
- Reserved-verb affordance: is greyed-in-palette-with-a-reason enough, or should reserved keys, when
  pressed bare, pop a "needs command log" which-key hint too?
- `q` = pop carries a strong quit/close cross-app meaning (convention dinged 5→4). Accept, or is there
  a better pop alternate than `h`/`q`?
- The residual `Esc` ≠ "back" negative-transfer risk right after `Enter`-dive — accepted (teach pop =
  `h`), but watch in the eyeball pass.
- Does the walkthrough-demo acceptance artifact live in the repo (a committed asset / gallery), or is
  it an in-session eyeball like the axis-inset gallery?

---

**Next step:** `/orb:spec` to crystallise this into a structured specification (or `/orb:card` first
to formalise card 0018). The scope-axis decision (selection-first) is the load-bearing input; the
scored candidate keymap in `keymap-research.md` is the design substrate.
