# Keymap ergonomics research — digest

Derived by a 12-agent workflow (study → synthesize → 3× adversarial critique → reconcile). Visual
cheat-sheet in `keymap-candidate.html` (artifact:
https://claude.ai/code/artifact/8a9181d3-36e8-4bd0-903b-c80a8d6157e7). Full workflow output lives in
the session transcript, not the repo.

> **v1 scope note (added after the /orb:spec adversarial review).** The candidate below is the full
> scored design and stays the source of truth for *bindings and rationale*. But the review found the
> cross-filter verbs `f` / `g f` / `t` have **no keyboard predicate source** on shipped code
> (`propagate_selection` needs a pointer-derived `Predicate`), so **v1 DEFERS `f` / `g f` / `t`** — and
> with them the `g`-prefix wiring, which-key, and pending-prefix handling (their first live use ships
> with the deferred keyboard data-target). The **mark-altitude floor is CUT** (v1 focus = dashboard→view).
> `c` = set-colour-scheme becomes **`c` = cycle-colour-scheme** on the focused view's sequential-scheme
> marks (transient). The `g`-broadcast *mechanism* stays implemented + unit-tested in the resolver.
> A second pass (`/orb:review-spec`) then **confirmed `/` (focus-jump = jump-to-component, per row
> below) and `?` (help overlay) stay in v1** — wired in the spec, resolving a scope gap where
> they were scored v1 here but neither built nor deferred in the spec. See `spec.yaml` (v1.2) for the
> ratified v1 boundary.

## v1 shipped-binding provenance

Every key bound in the shipped registry (`brightfield-keys`) traces to a row here — no key is bound by
taste. The mechanical cross-ref `brightfield_keys::registry()` bound-longnames ↔ this table is asserted
by the provenance test; scores mirror `VerbEntry.scores` (frequency / mnemonic / convention,
1–5, with a motor-cost note).

| longname | key(s) | freq / mnem / conv | motor note |
|----------|--------|--------------------|------------|
| `dive-in` | `l` · `enter` | 5 / 4 / 5 | home-row `l` = right/in (ranger, miller-columns) |
| `pop-out` | `h` · `q` | 5 / 4 / 5 | home-row `h` = left/out (ranger, miller-columns) |
| `focus-next-sibling` | `j` · `tab` | 5 / 4 / 5 | home-row `j` = down/next (vim) |
| `focus-prev-sibling` | `k` · `shift-tab` | 5 / 4 / 5 | home-row `k` = up/prev (vim) |
| `toggle-focus` | `cmd-e` | 3 / 3 / 3 | cmd-e = editor swap; free of gpui-component Input's chord set |
| `focus-jump` | `/` | 3 / 4 / 5 | `/` = search/jump (vim, less) — jumps focus to a component |
| `open-palette` | `space` · `cmd-shift-p` | 5 / 5 / 5 | space = palette (helix, which-key); cmd-shift-p global twin (VS Code) |
| `open-help` | `?` | 2 / 4 / 5 | `?` = help (near-universal convention) |
| `clear-selection` | `escape` | 4 / 4 / 5 | esc = cancel/clear (universal); terminal rung of the Esc ladder |
| `reload-spec` | `cmd-r` | 2 / 4 / 4 | cmd-r = reload (browser); bare `r` NOT bound (dirty-guard) |
| `open-home` | `cmd-shift-h` | 3 / 5 / 4 | cmd-shift-h = home; free of the editor chord set; keeps your place under Continue |
| `toggle-presentation` | `p` | 2 / 3 / 3 | `p` = present (shipped fixed point) |
| `save-spec` | `cmd-s` | 3 / 5 / 5 | cmd-s = save (universal; shipped, editor-scoped) |
| `cycle-colour-scheme` | `c` | 3 / 5 / 3 | `c` = colour (mnemonic); view-scoped, transient preview |
| `toggle-outline-rail` | `cmd-b` | 4 / 2 / 5 | cmd-b = the left dock (Zed `workspace::ToggleLeftDock`, VS Code `toggleSidebarVisibility`); round-trip focus, never a numeric |

### Navigating the frame — pan, zoom, axis lock, reset

Added when the navigation extent landed. Scored against the same three axes as every row above,
and against one extra constraint this family has and the others do not: **the chart grammar's
home row is already spoken for.** `h`/`j`/`k`/`l` move FOCUS across the component tree, and a
family that borrowed them for panning would give one key two meanings resolved by an invisible
mode. So the frame verbs take the arrow keys, which every map, canvas and image viewer already
uses for exactly this, and which no bound verb in this registry claims. Convention scores of 5 are
earned rather than assumed: panning by arrow key and zooming by `+`/`-`/`0` are the two most
widely shared bindings in software that shows a viewport.

Direction is four verbs rather than one parameterised verb because the registry has no
parameterised verbs — a longname resolves to an action, and inventing an argument channel for this
family alone would be a bigger change than four rows.

| longname | key(s) | freq / mnem / conv | motor note |
|----------|--------|--------------------|------------|
| `pan-left` | `left` | 4 / 5 / 5 | arrow keys = pan (every map and canvas); free of the `hjkl` focus grammar |
| `pan-right` | `right` | 4 / 5 / 5 | arrow keys = pan (every map and canvas); free of the `hjkl` focus grammar |
| `pan-up` | `up` | 4 / 5 / 5 | arrow keys = pan (every map and canvas); free of the `hjkl` focus grammar |
| `pan-down` | `down` | 4 / 5 / 5 | arrow keys = pan (every map and canvas); free of the `hjkl` focus grammar |
| `zoom-in` | `=` | 4 / 5 / 5 | `=` is the unshifted `+` (browsers, maps, editors); no shift reach for the common direction |
| `zoom-out` | `-` | 4 / 5 / 5 | `-` = zoom out, the universal twin of `+`; adjacent to `=` on every layout |
| `cycle-axis-lock` | `x` | 2 / 4 / 3 | `x` = axis (mnemonic); a view-scoped mode toggle, free of the shipped chart grammar. Cycles both → x → y |
| `reset-extent` | `0` | 3 / 4 / 5 | `0` = reset zoom (browsers `cmd-0`, maps); bare here because the chart owns the digit row. Separate from `escape`: a brush and a frame are different state |

### Protocol altitude — the asset-graph grammar

The protocol panel is a distinct altitude, so its verbs never collide with the chart grammar. Motion,
folds, and drill are **View**-tier (never logged); the object verb that names an asset is **Data**-tier
(logged by longname + dotted address, never a screen position). The topological verbs survive re-layout
because they walk the graph's edges, not pixel geometry.

| longname | key(s) | freq / mnem / conv | motor note |
|----------|--------|--------------------|------------|
| `protocol-producer` | `h` | 5 / 4 / 5 | `h` = upstream/producer (vim left); topological, survives re-layout |
| `protocol-consumer` | `l` | 5 / 4 / 5 | `l` = downstream/consumer (vim right); topological, survives re-layout |
| `protocol-sibling-next` | `j` | 5 / 4 / 5 | `j` = next rank sibling (vim down); orders by node id within the layer |
| `protocol-sibling-prev` | `k` | 5 / 4 / 5 | `k` = prev rank sibling (vim up); orders by node id within the layer |
| `toggle-fold` | `z a` | 3 / 4 / 5 | `za` = toggle fold (vim fold family); one verb over both folds (family members, or a `sql:` step's CTEs), resolved by what the cursor is on; a fold is a view change, never logged |
| `protocol-drill-in` | `enter` | 4 / 4 / 5 | enter = dive (miller-columns); pushes the drill stack |
| `protocol-drill-out` | `escape` | 4 / 4 / 5 | esc = pop one level (Esc ladder); breadcrumb tracks the pop |
| `open-steps-sheet` | `shift-s` | 3 / 5 / 4 | `S` = steps sheet (VisiData sheet family); answers "where is my step list" |
| `yank-address` | `y` | 3 / 5 / 4 | `y` = yank (vim); a Data verb — logged by longname + dotted address |

> **Rename note.** The `z a` row was scored and recorded as `toggle-fold-family`, when the only thing
> that folded was a parameterised family. The verb was later broadened to also open a `sql:` step's
> CTEs under the cursor, so the row is now `toggle-fold`: same key, same scores, same rationale — a
> longname that no longer named half of what the key does was a help sheet and a palette that lied.

Reserved verbs are deliberately unscored (no key yet), shown greyed in the palette until their keys land:
needs-keyboard-target (`filter-view`, `cross-filter-all`, `toggle-point-select`, `set-param`) and
needs-command-log (`change-mark-type`, `add-mark`, `set-channel`, `remove-mark`, `undo`).

Recorded scope-model decisions (the manual-review half): selection-first (scope = focused node);
`g` = dashboard-broadcast mechanism, runtime verbs only, always resolves to root; `z` dropped;
view-altitude floor (no mark descent); `f`/`g f`/`t` deferred. These match the shipped resolver in
`brightfield-keys` (`scope.rs`, `focus.rs`).

---

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
