Discovery: **Conditional encoding v1 — Mosaic `highlight`, not Altair `condition`** (grammar-direction research, 2026-07-13). Answers the one strategic fork the Altair interaction gap-map (`2026-07-12-altair-interaction-grammar-gaps.md`, gap #1) deferred to `/orb:discovery`: which grammar does Brightfield adopt for selection-driven conditional encoding? Feeds card 0021.

## The fork

Two grammars can express "encode this channel differently for data inside vs outside a selection":

- **Mosaic `highlight` interactor** — `{select: highlight, by: $sel, opacity?/fill?/stroke?}` deemphasizes the non-matching set. Already in Brightfield's vendored corpus (splom, weather, region-tests, …).
- **Vega-Lite / Altair `condition` object** — a per-channel `{condition: {param|test, value|field}, value|field}` that resolves any channel to a different value (or field) based on a predicate. This is what the gap-map memo assumed as the target shape.

The memo flagged this "the load-bearing primitive… biggest lift" and correctly gated it on discovery. The decisive unknown: does adopting Altair's `condition` diverge from the Mosaic specs Brightfield sells portability on?

## Method + provenance (honest)

Ran an 8-agent research+verify workflow (`wf_30b356ff-436`). **The four research-phase agents misfired** — each burned real web calls (16–25 tool calls) but returned schema-isolation placeholder stubs, and one hit the StructuredOutput retry cap. That phase produced nothing usable. The finding therefore rests entirely on **three adversarial verify agents**, each with confirmed working web access (WebSearch + WebFetch + GitHub code search), each independently tasked to *refute* the claim, plus direct codebase grounding. All three failed to refute, at high confidence, with mutually-corroborating primary-source citations. Not a substitute for a clean run — but the citations are checkable and consistent.

## Verified findings (primary sources)

1. **Mosaic/vgplot has NO per-channel `condition` grammar.** Absent from the [interactors API](https://idl.uw.edu/mosaic/api/vgplot/interactors.html), [marks API](https://idl.uw.edu/mosaic/api/vgplot/marks.html), and [declarative spec format](https://idl.uw.edu/mosaic/api/spec/format.html). GitHub code search over `uwdata/mosaic` finds `condition` **only** in the SQL layer (JOIN / CASE) — zero encoding-layer hits. Mosaic channels are field-or-constant (or `{sql: …}`); no when/then/otherwise object exists on any channel.

2. **`highlight` is the idiomatic, current, corpus-wide selection→emphasis interactor.** Interface (`packages/vgplot/spec/src/spec/interactors/Highlight.ts`): `{ select:'highlight'; by; opacity?(default 0.2); fillOpacity?; strokeOpacity?; fill?; stroke? }`. It **deemphasizes the non-matching set** — docs: *"Selected values keep their normal appearance. Unselected values are deemphasized."* Implementation (`Highlight.js`) is a whole-mark boolean SVG-attribute toggle: `node.setAttribute(attr, t ? base : value)`. Used across splom / weather / wind-map / line-multi-series / facet-interval / region-tests. Canonical: `vg.highlight({by: $brush, opacity: 0.1})`.

3. **VL `condition` is strictly more general** — it can bind a channel to a DIFFERENT FIELD, chain multiple branches, route through the channel's scale, express `test` (data-driven) predicates, and carries the `empty: true|false` knob. `highlight` is the boolean opacity/fill/stroke subset for the "emphasize by muting the rest" use case only. [[condition.html](https://vega.github.io/vega-lite/docs/condition.html)]

## Decision (Hugh, 2026-07-13): **highlight-first v1**

Adopt the Mosaic-native `highlight` interactor for v1; **defer** the general Altair `condition` grammar until authoring demand for restyle-the-selected / different-field / chained conditionals materialises. Rationale: portable to the Mosaic corpus (a stated product value — `condition` has zero corpus presence), reuses an already-present render primitive (small lift), and unblocks vendored specs. The general `condition` grammar remains a legitimate later, discovery-gated step.

## Brightfield grounding (our code)

- **Render primitive EXISTS but is test-only.** `HighlightState` + `apply_highlight` (crates/brightfield-render/src/mark.rs:19-28, 203-215) reach the scene via `ChartData.highlight` (scene.rs:45-47) — but every production `ChartData` sets `highlight: None` (scene.rs:485..); only a test constructs one (scene.rs:974/986). So the render dim is built; the **declarative `select: highlight, by: $sel` → production `selection_state` binding is what's missing.**
- `highlight` is already vocab-`Implemented` (crates/brightfield-spec/src/vocab.rs:230) — on the strength of that test-wired primitive.
- **Selection substrate is complete:** `compile_selection` → `Predicate` → `propagate_selection` → coordinator re-query/re-scene (crates/brightfield-sql/src/lower.rs; crates/brightfield-engine/src/lib.rs:309; crates/brightfield-ui/src/crossfilter.rs:306). `highlight` rides it for free — a SELECT-list membership flag, not a WHERE (rows are dimmed, not dropped).
- **Two generalisations the v1 needs:** (a) parse the declarative interactor params (`by`/`opacity`/`fill`/`stroke`); (b) generalise `apply_highlight`'s FIXED alpha-dim into the `opacity`/`fill`/`stroke` override surface.

## Corrections this discovery made to the 2026-07-13 scoping investigation

- "highlight machinery dormant / render-only test-wired" → precise version: the render **primitive** exists (test-only); the **declarative binding** is what's dormant. The lift is parse + wiring + honouring overrides, not new rendering — but not a one-line switch either.
- "wiring highlight unblocks splom / weather / region-tests" → `region-tests` **also** uses `select: region`, which is `Unimplemented` (vocab.rs:231). Highlight alone unblocks **splom + weather**; region-tests needs `region` too.
- highlight only **mutes the non-selected set** — it does not restyle the selected set. "Colour the selected, grey the rest" is expressed by *greying the rest* (`fill: '#ccc'`, per weather.yaml); the selected keep their base colour. Actively recolouring the *selected* set is beyond `highlight` → `condition` territory.

## Handoff → card 0021, open for `/orb:design`

- **`empty` default:** empty selection → all-normal vs all-dimmed (the Mosaic/VL default matters for how a fresh, unbrushed dashboard reads).
- **Membership source:** `apply_highlight` takes a per-row predicate today — build it from the selection geometry (render-side closure, no DB round-trip) or a SQL membership column (one source of predicate truth)? For `highlight` specifically the render-side closure likely suffices — smaller than the general-condition SQL path.
- **`region` bundling:** highlight-only v1 (splom + weather) vs bundle the `region` interactor to also unblock region-tests.
- **Literal-colour value for `fill:` override:** confirm the interactor-param path sidesteps the "marks have no literal-colour channel" prerequisite the scoping investigation flagged (highlight's `fill` is an interactor param, not a mark channel — likely a non-issue, but verify in design).

## Citations

- https://idl.uw.edu/mosaic/api/vgplot/interactors.html
- https://idl.uw.edu/mosaic/api/vgplot/marks.html
- https://idl.uw.edu/mosaic/api/spec/format.html
- https://idl.uw.edu/mosaic/examples/splom.html
- https://github.com/uwdata/mosaic/blob/main/packages/vgplot/spec/src/spec/interactors/Highlight.ts
- https://raw.githubusercontent.com/uwdata/mosaic/main/packages/vgplot/plot/src/interactors/Highlight.js
- https://raw.githubusercontent.com/uwdata/mosaic/main/specs/yaml/splom.yaml
- https://vega.github.io/vega-lite/docs/condition.html
