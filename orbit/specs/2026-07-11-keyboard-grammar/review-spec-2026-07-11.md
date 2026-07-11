# Spec Review

**Date:** 2026-07-11
**Reviewer:** Context-separated agent (fresh session)
**Spec:** 2026-07-11-keyboard-grammar (v1.1)
**Verdict:** REQUEST_CHANGES

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 1 |
| 2 — Assumption & failure | MEDIUM finding + content signals (cross-system boundary to gpui-component/zed; shared keymap-as-data config) | 3 |
| 3 — Adversarial | not triggered (no cascading/contradicted-assumption concerns; every load-bearing premise code-verified TRUE) | — |

**Note on resolution.** `orbit spec resolve` / `spec show` fail here — the CLI reads `.orbit/specs/` (dot-prefixed) but this repo's specs live in `orbit/specs/`. Reviewed the `spec.yaml` on disk directly. `ac_type: gate` is used in place of a `gate:` boolean; the sole gate (ac-15) passes the deterministic Pass-1 gate-description check (non-empty, not a placeholder token, well over 20 chars).

**Premises spot-checked against the actual pins (all accurate):** `scripts/compare-example-pngs.sh` exists (ac-15); `clear_pending_keystrokes` is `pub(crate)` and `has_pending_keystrokes` is `pub` at zed 1d217ee window.rs:4935/4931 (Esc replay-on-non-match pivot grounded); gpui-component Input binds `secondary-enter` at state.rs:169 (spec cited :168 — off by one; cmd-Enter collision is real); `configured_renderer` at mark.rs:2959 (the `c`-scope predicate); `parent_plot` at analysis.rs:112 with the documented containment semantics; `PlacedChart` at chart_view.rs:40 holds x/y/w/h/state/coordinator and no ComponentPath (the greenfield seam is real); no existing help-sheet or search-jump wiring in brightfield-app/brightfield-ui (relevant to Finding 1). This is a rigorously code-grounded spec.

## Findings

### [MEDIUM] `/` search-jump and `?` help-keys sit in a scope gap — neither built nor deferred
**Category:** missing-requirement
**Pass:** 1
**Description:** The scored v1 nav spine in `keymap-research.md` (the "v1 — nav & palette spine" table) lists `/` (search-jump, 3/4/5) and `?` (help-keys, 2/4/5) as "v1 build", and the interview success-criteria loop names `/`. The spec then leans on `?` as present: the constraint at spec.yaml:88-90 lists `?` (help) among "shipped/convention-locked fixed points" the keymap must "design AROUND", and ac-01 has the registry PRODUCE a help sheet. But no acceptance criterion wires `?` to open that help sheet, and none wires `/` search-jump. Neither appears in the DEFERRED constraint (spec.yaml:77-82), which enumerates f/gf/t/set-param + g-prefix/which-key/pending + m/a/e/d/undo — not `/` or `?`. ac-17's walkthrough exercises neither. So both fall through: not built, not deferred, not reserved.
**Evidence:** spec.yaml constraint:88-90 treats `?` (help) as a fixed point, yet a grep of `brightfield-app`/`brightfield-ui` finds no existing help-sheet or search-jump surface — so `?`/help is neither shipped nor built this card, contradicting its "fixed point (help)" framing. help-keys is also absent from the two reserved vocab sets in ac-01 (needs-command-log / needs-keyboard-target), so it isn't even palette-reserved. The spec is otherwise meticulous about listing every deferral; this omission reads as an oversight, not a decision.
**Recommendation:** Pick one and state it explicitly: (a) build them — add a wiring AC for the `?` help overlay (the discoverability layer-3 the research relies on) and decide `/` search-jump; or (b) defer them — add `/` and `?` to the DEFERRED/reserved-and-visible set with a reason, drop `?` from the "fixed points" list, and note the discoverability story rests on Space + breadcrumb alone in v1. Either resolves the implementer's "do I wire these?" ambiguity.

### [LOW-MEDIUM] ac-07 verifies the dispatch MODEL, not GPUI's conformance to it — "capability-verified by ac-07" overstates the headless guarantee
**Category:** test-gap
**Pass:** 2
**Description:** ac-07 is `ac_type: code`, `verifies: capability` (headless-unit-tested). It asserts a gpui-free projection of the keymap-as-data vec and holds invariants like "context=None resolves from BOTH workspace and editor contexts" and "while the EDITOR context is focused NO workspace-scoped bare verb resolves". Those are facts about GPUI's runtime context-resolution semantics, encoded as assumptions inside the model. A unit test over the projection proves the projection equals the shipped binding vec and is internally self-consistent — but a *wrong* encoding of GPUI's resolution rules would still pass. Only the eyeball ACs (ac-09, ac-12) actually exercise real GPUI dispatch. Yet ac-09 and ac-12 each say "the ... invariant is capability-verified by ac-07", which reads as if the headless test de-risks the GPUI-semantics assumption. It de-risks projection fidelity + self-consistency, not GPUI conformance.
**Evidence:** spec.yaml ac-07 verification ("Unit tests over the projection assert it equals the shipped binding vec and hold each invariant"); ac-09:207-209 and ac-12:243-244 attribute the live invariants to ac-07's capability verification.
**Recommendation:** Reword so the split is honest: ac-07 proves the resolution table is a faithful projection of the binding vec and internally consistent; the *conformance* of GPUI's live dispatch to those context/overlay invariants is eyeball-verified by ac-09/ac-12. No scope change — just don't let "capability-verified by ac-07" imply the runtime semantics are headless-proven.

### [LOW] ac-17 pins "cmd-Enter to the editor" while ac-09 leaves the toggle chord open
**Category:** constraint-conflict
**Pass:** 2
**Description:** ac-09 and the constraint at spec.yaml:55-59 correctly establish that cmd-Enter is NOT free (Input binds `secondary-enter`) and give the implementer two branches: pick a chord Input does not claim, OR bind NoAction on `secondary-enter` and keep cmd-Enter. ac-17's walkthrough, however, hard-codes "cmd-Enter to the editor" as a literal sign-off step. If the implementer takes the "pick an unclaimed chord" branch, ac-17's recorded artifact will use a different chord than its own text specifies.
**Evidence:** spec.yaml ac-09:200-204 ("genuinely unclaimed by Input, or Input's secondary-enter NoAction-overridden") vs ac-17:293 ("cmd-Enter to the editor").
**Recommendation:** In ac-17, refer to "the focus-toggle chord" rather than pinning `cmd-Enter`, or resolve the chord choice in the spec (e.g. commit to the NoAction-override branch so cmd-Enter is authoritative).

### [LOW] macOS-eyeball-only verification vs macOS/Linux/Windows targets; cross-platform accelerator/modifier risk has no AC
**Category:** content-signal
**Pass:** 2
**Description:** All live ACs (ac-09..14, ac-16, ac-17) are macOS-eyeball. The product targets macOS/Linux/Windows (CLAUDE.md), and the spec itself notes the toggle modifier diverges per platform ("secondary-enter = cmd-Enter on macOS / ctrl-Enter elsewhere", spec.yaml:56-57). The constraint also flags that the global cmd-s/cmd-r rebind must "audit the native menu accelerators for clashes across macOS/Linux/Windows" — but that audit is not captured by any AC. So Linux/Windows keyboard, focus-toggle-modifier, and native-menu-accelerator behaviour ship unverified this card.
**Evidence:** spec.yaml:53-54 and :56-57 (per-platform modifier + accelerator audit) with no corresponding gate/AC; every live AC states "manual macOS eyeball".
**Recommendation:** Consistent with the project's established macOS-eyeball convention, so not a blocker — but record the decision explicitly: either add the cross-platform accelerator audit as a (possibly deferred) check, or state in scope that Linux/Windows keyboard verification is out of scope for v1 and tracked for a follow-up. Avoids a silent cross-platform regression surfacing at consumer-delivery time.

---

## Honest Assessment

This is one of the more rigorous specs I've reviewed: the scope was already reshaped by a prior adversarial, code-verified review (v1.1 defers f/gf/t for want of a keyboard predicate source, cuts the mark-altitude floor, corrects the Esc mechanism and the `c` scope), and every load-bearing code pin I independently checked was accurate. The architecture is sound and buildable on shipped code, the framework-free/eyeball verification split matches project convention, and the transient/no-write discipline (ac-13, exit_conditions) doubles as the rollback story (nothing durable is persisted; the PNG byte gate ac-15 is the safety net). The single change worth making before implementation is closing the `/`+`?` scope gap (Finding 1) — a small, bounded decision that currently leaves an implementer guessing whether the search-jump and help surfaces are in v1, made confusing by the constraint list still treating `?` as an existing "fixed point" when no help surface ships. The other three findings are wording/consistency tightenings, not structural risks. Resolve Finding 1 and this is ready.

---

## Resolution — spec v1.2 (2026-07-11)

All four findings folded; spec bumped v1.1 → v1.2.

- **Finding 1 (MEDIUM) — RESOLVED, build both** (Hugh's call). Added **ac-18 focus-jump (`/`)** — a gpui-free path-fuzzy-match over the ComponentPath tree (reuses the ac-05 matcher on path labels) plus live `/` overlay that moves focus to the chosen node; distinct from the Space palette by design (`/` finds NODES, palette finds VERBS). Added **ac-19 help overlay (`?`)** — renders the registry-produced help sheet (ac-01), Esc-dismissable, read-only. The constraint list no longer frames `?` as a phantom shipped "fixed point": `?`/`/` are now convention-locked-and-delivered-this-card (alongside Space/Enter), the surface built behind each. Walkthrough (ac-17) now exercises both. keymap-research.md's v1-scope note updated to confirm `/`+`?` stay in v1.
- **Finding 2 (LOW-MEDIUM) — RESOLVED.** ac-09 and ac-12 reworded: ac-07 proves the resolution table is a faithful PROJECTION of the shipped binding vec and internally consistent; GPUI's live CONFORMANCE to those context/overlay invariants is what the eyeball ACs verify — the headless test does not prove the runtime semantics.
- **Finding 3 (LOW) — RESOLVED.** ac-17 now says "the focus-toggle chord" instead of pinning cmd-Enter, matching ac-09's open branch.
- **Finding 4 (LOW) — RESOLVED.** Added a cross-platform-scope constraint recording that Linux/Windows keyboard behaviour, the per-platform toggle modifier, and the native-menu accelerator audit are out of v1 verification scope (macOS-eyeball convention), tracked for the consumer-delivery follow-up.
- **Bonus:** the reviewer's off-by-one — Input binds `secondary-enter` at input/state.rs **:169**, not :168 — corrected in the constraint and implementation note.

Verdict cleared: spec is implementation-ready.
