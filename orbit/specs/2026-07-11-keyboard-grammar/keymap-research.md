# Keymap ergonomics research — digest

Derived by a 12-agent workflow (study → synthesize → 3× adversarial critique → reconcile). Full
output in `keymap-research-raw.json`; visual cheat-sheet in `keymap-candidate.html`
(artifact: https://claude.ai/code/artifact/8a9181d3-36e8-4bd0-903b-c80a8d6157e7).

Objective Hugh set: **balance frequency + mnemonic + convention, show the conflicts.** A 4th
motor-cost lens (KLM base × carpalx-style ergonomic multiplier) is folded into each score's conflict
note rather than collapsed into one scalar (so the three named axes stay visible).

---

## Methodology (design principles, priority order)

1. **Frequency-stratified assignment + explicit motor-cost lens.** `C = Σ freqᵢ·costᵢ`; tier-1 hot
   verbs get cheap, well-placed keys (motor wins, mnemonic irrelevant under the power law); cold-tail
   verbs get recall-strong convention/mnemonic keys; genuine conflicts live in the mid-band. Do NOT
   collapse to one number — it would hide the conflicts Hugh wants to see.
2. **Selection-first spine; scope is where focus sits; one flat runtime-broadcast prefix `g`, no `z`.**
3. **`g` has exactly one meaning/class — runtime scope-widening only, always resolves to root.**
   Structural edits are never `g`-prefixed (pop focus + bare verb instead).
4. **Context-scope every canvas verb; correct the false "global chord" premise.** `cmd-s` is
   editor-scoped (`shell.rs:90`), not global. Anything that must fire regardless of focus binds
   `context=None` on `WorkspaceRoot`.
5. **The focus model is a first-class v1 deliverable, not an assumption** (canvas focus is mouse-only).
6. **Uniform off-altitude policy; bare-key applicability mirrors the palette** (same predicate; an
   off-altitude bare verb rejects with a visible reason — never silent no-op, never auto-walk).
7. **Honour convention-locked + in-house fixed points** (Esc, cmd-s, `/`, `?`, Space, Enter, `u`, `^`,
   `-`, `=`, `#/%/@`, and the shipped bare `p`). Negative transfer is worse than an arbitrary key.
8. **Esc precedence ladder:** dismiss overlay › cancel pending prefix (`clear_pending_keystrokes`) ›
   clear selection. Does not auto-pop.
9. **Tree-navigation convention (ranger/miller), not vim screen-direction:** `h` out, `l` in, `j/k`
   siblings; `Enter`/`q` VisiData alternates. The ComponentPath tree has no consistent screen
   projection (hconcat vs vconcat), so no key promises a screen direction.
10. **Palette-first; mnemonics are opportunistic, not generative.** Build the palette first; it makes
    cryptic-but-fast keys affordable and shows each row's key inline. Leave gaps rather than
    manufacture a bad mnemonic.
11. **Build-readiness gates v1; the ceiling is two deferred builds (command log AND focus model).**
    Reserved verbs stay visible (greyed) so the vocabulary is taught before the keys exist.

---

## Scope-axis recommendation: **selection-first**

Scope = where focus sits on the `ComponentPath` tree. Navigate focus (`l`/`Enter` dive, `h`/`q` pop,
`j`/`k` siblings); a bare argumentless verb acts on the focused node at whatever altitude it occupies.
Down / lateral / choosing-which-altitude-to-edit are all done by focus, not a per-verb prefix. The
sole non-focus scope affordance is `g` = broadcast a runtime verb across the whole dashboard (flat
root-scope widen — safe at every altitude, never "delete my parent", runtime-only). `z` is **dropped**
(dive already gives containment-down more cheaply; frees `z` for future zoom; removes the `z`=zoom
collision).

**Evidence:** Helix models scope as movement over a real syntax tree (`Alt-o` parent, `Alt-i` child,
`Alt-n/p` sibling) — 1:1 with `root/vconcat[i]/plot[j]/mark[k]`. Zed/GPUI expresses scope as
context-tree position (deeper-context-wins), so a bare verb already acts on the focused node.
VisiData's own `g` is not pure containment (`s`=current row, `gs`=all rows) — honest proof a scope
prefix can't stay pure across verbs, which is why `g` is confined to one runtime class here.
ranger/miller-columns supply the `h/l`+`j/k` convention. Gmail/Linear/vim `g`="go to" warn that the
non-standard `g`=broadcast must be carried by a visible breadcrumb + which-key.

---

## Candidate keymap (scored; F/M/C are 0–5)

### v1 — nav & palette spine (this card builds; rides GPUI dispatch + the new focus model)

| Key | Verb | Scope behaviour | F/M/C | Status |
|---|---|---|---|---|
| `Space` / `⌘⇧P` | command-palette | fuzzy longname+help, focus-scoped, freq-ranked | 5/3/5 | v1 build |
| `⌘↩` | editor⇄canvas focus-toggle | **gates the whole grammar** (canvas focus is mouse-only) | 5/3/3 | v1 build |
| `l` / `↩` | dive-surface | descent stops at **mark** in v1 | 5/4/5 | v1 build |
| `h` / `q` | pop-surface | `q` convention dinged (quit/close) | 5/3/4 | v1 build |
| `j` / `k` | focus next/prev sibling | ranger convention; `Tab`/`⇧Tab` free alternates | 5/3/4 | v1 build |
| `/` `?` | search-jump · help-keys | uncontended conventions; `?` states the g-rule | 3/4/5 · 2/4/5 | v1 build |

### v1 — runtime verbs (drive already-built dispatch; no command log needed)

| Key | Verb | Scope behaviour | F/M/C | Status |
|---|---|---|---|---|
| `f` · `g f` | filter-view / cross-filter-all | bare=this view; `g f`=whole dashboard. *flag: f=filter ≠ VisiData f=fill — kept* | 5/4/2 | built |
| `t` | toggle-point-select | matches toggleX/toggleY interactor names | 4/5/4 | built |
| `Esc` | clear-selection | ladder rung 3; does not auto-pop | 4/3/5 | built |
| `p` | toggle-presentation | **shipped** bare `p`; immovable | 4/5/5 | shipped |
| `⌘S` · `⌘R` | save · reload spec | re-bind global; `⌘R` off bare `r` + dirty-buffer guard | 4/4/5 | built* |
| — | set-param | arrow-nudge on focused widget / palette value (no bare key) | 4/3/3 | built |

### reserved — needs command log (visible greyed in palette, unbound)

| Key | Verb | Why it waits | F/M/C | Status |
|---|---|---|---|---|
| `u` | undo | scores highest yet **cannot exist before the log** — the tell that no spec-edit verb precedes it | 4/5/5 | blocked |
| `c` | set-colour-scheme | **v1 early live-preview** via renderer_override (transient; persisting still waits) | 3/5/3 | early ✓ |
| `m` | change-mark-type | structural spec edit | 4/5/3 | reserved |
| `a` · `e` | add-mark · set-channel | structural; add-at-parent = pop + bare `a` (never `g a`) | 4/4/4 · 4/3/2 | reserved |
| `d` | remove-mark | destructive — must not bind before undo; structure rides focus so `g` never deletes parent | 3/4/4 | blocked |
| — | zoom-extent | `z` freed as future home once a gesture caller exists (`+`/`-` dropped: self-collide with `=`/`-`) | 2/2/3 | future |

*`c` = set-colour-scheme is the one authoring verb pulled into v1 (Hugh's choice), transient-preview only.*

---

## Palette & discoverability

**Palette** (`Space` in WORKSPACE; `⌘⇧P` global on `WorkspaceRoot`): fuzzy over longname+help,
focus-scoped by the same `scope_applicability` predicate the bare keys use, frequency-ranked + a
per-user recency counter, **each row shows its bound key inline** (the recognition→recall handoff).
Anchors **outside** the WORKSPACE subtree (canvas doesn't stop propagation) — add a focus-invariant
test: no bare verb dispatches while the palette is open. Sigil sub-modes in one field: bare text = verb
by name; `@` = jump to component (keyboard route = search-jump on `/`); `#`/`:` = data/column layer.
Reserved verbs greyed with "needs command log". A persistent "`g …` = same verb, whole dashboard"
affordance teaches the rule, not just the `gf` instance.

**Discoverability layers** (additive, expert path never lengthened): (0) the editor↔canvas focus
toggle, surfaced in menu + help (it gates everything); (1) the palette; (2) a which-key overlay after
`g` (and proactively on first bare `f`), using GPUI's ~1s pending engine; (3) the `?` help sheet
(every verb: longname, bindings, scope, help); (4) an always-visible breadcrumb + focus ring showing
altitude, pending prefix, and a g-verb's resolved root target; (5) the menu bar mirroring the palette.

---

## Reference lessons (how each model handles scope + top transfer/pitfall)

- **VisiData** — orthogonal stackable prefixes (`g`=embiggen/outward, `z`=zmallify/finer). *Transfer:*
  keystroke→longname decoupling + keymap-as-data. *Pitfall:* `z` is semantically overloaded
  (cell/scroll/typed/number/keep-priority) — relearned per verb; ours must mean one thing or drop it.
- **vim** — scope is a noun chosen *after* the verb (motion reach / count / text-objects). *Transfer:*
  frequency-weighted placement over mnemonic purity for the hottest few (hjkl). *Pitfall:* don't bolt
  verb-then-scope (operator-motion) onto our prefix-first grammar — opposite parse directions.
- **Helix/Kakoune** — selection-first; the selection IS the scope; hierarchical grow/shrink over a
  tree-sitter tree. *Transfer:* make focus a persistent highlighted selection + argumentless verbs
  (the model we adopted). *Pitfall:* don't copy vim-colliding remaps; lean on domain mnemonics.
- **Zed/GPUI** — scope = context-tree position + attribute flags (deeper-context-wins). *Transfer:*
  `.key_context()` per node + `>` descendant predicate map 1:1 onto ComponentPath. *Pitfall:* the
  editor `Input` swallows all bare printable keys + a large chord set (cmd-a/c/v/x/z, cmd-shift-z,
  cmd-f, cmd-., cmd-]/[, tab, arrows, escape, enter) — the collision surface for global chords.
- **Modern apps (Gmail/Linear/Superhuman/Sublime/VSCode)** — selection-first noun→verb + sigil
  namespaces in one palette field. *Transfer:* palette renders the resolved key inline + fuzzy on
  longname (nearly free for us). *Pitfall:* `g`="go to" convention conflict (surfaced as a decision).
- **HCI (KLM/GOMS)** — no scope primitive, but: broad-shallow beats narrow-deep (keep the prefix
  alphabet tiny); a silent prefix is a transient mode (make it self-revealing); per-key motor-cost
  table (home-row strong ×1.0 → Shift/Ctrl +1 K each + RSI flag). *Pitfall:* optimising pure motor
  cost ignores the ~1.35s mental-recall M term where bad mnemonics actually bleed.

---

## Brightfield command-frequency model (38 verbs)

Tiers estimate KLM/GOMS expert cost for a dashboard **author**. **Key asymmetry:** frequency ranking
and build-readiness do NOT coincide — several tier-2/3 verbs are already-built runtime ops
(filter/clear/set-param/save/reload/toggle-presentation/zoom) bindable today, while most tier-2..4
authoring edits are greenfield pending the spec-edit representation. Reserve the cheapest un-taken
keys for the built verbs an author can exercise now.

- **Tier 1 (innermost loop, every few seconds):** focus-sibling next/prev, dive, pop, command-palette.
- **Tier 2 (per-mark core):** set-channel, change-mark-type, add-mark, filter-view, clear-selection,
  set-param, save-spec.
- **Tier 3 (per-chart occasional):** set-colour-scheme, add-view, remove-mark, toggle-point-select,
  zoom-extent, toggle-presentation, reload-spec, undo, rename/hide/derive-column.
- **Tier 4 (tuning, once/twice per chart):** set-inset/bandwidth/scale-domain/thresholds,
  reverse-scale, add-interactor/param-widget/legend, pan-extent, type-column, search-jump,
  cancel-query, remove-view.
- **Tier 5 (rarest):** edit-meta, help-keys.

**Scope-axis note from the model:** the ComponentPath ladder (`analysis.rs:95`, `parent_plot()` at
`:112`) is a genuine containment tree. Verbs split into *containment* (re-target the same verb at a
different node: filter, clear, add/remove-mark-vs-view, set-channel, colour, columns ops, nav) and
*magnitude* (modulate a scalar: zoom, pan, inset, bandwidth, scale-domain, set-param). VisiData
overloads `z` across both; selection-first dissolves the split by putting containment on the focus
axis and keeping only `g`=runtime-broadcast.
