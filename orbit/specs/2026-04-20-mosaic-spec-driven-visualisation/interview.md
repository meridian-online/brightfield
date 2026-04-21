# Design: Mosaic spec-driven visualisation

**Date:** 2026-04-20
**Interviewer:** Nightingale (rally design sub-agent + lead)
**Card:** orbit/cards/0001-mosaic-spec-driven-visualisation.yaml
**Rally:** orbit/specs/rally.yaml (spec foundation; paired with card 0002)

---

## Context

Card: *Mosaic-spec-driven visualisation* — 5 scenarios; goal: "A Mosaic YAML spec defining a two-view cross-filtered dashboard renders interactively from the spec alone."
Prior specs: 0 (greenfield project)
Gap: The full design space — this card establishes the spec-as-stable-contract for the platform. Every downstream card (query engine, coordinator, renderer) reads against the AST and diagnostic model this card settles.

Reference material consulted:
- `/Users/hugh/github/meridian-online/brightfield-brief.md` (§2 principles, §3.1–3.3 architecture, §5 what Mosaic provides, §6 risks)
- `/Users/hugh/github/uwdata/mosaic/packages/vgplot/spec/` (canonical TypeScript schema + parser)
- `/Users/hugh/github/uwdata/mosaic/docs/public/specs/{yaml,json}/` (55 reference specs)

Decision pack: `orbit/specs/2026-04-20-mosaic-spec-driven-visualisation/decisions.md`.

## Q&A

### Q1: AST representation strategy
**Q:** How should brightfield model the parsed spec — a serde `Value` pass-through, a 1:1 port of Mosaic's OO AST classes, or Rust-idiomatic typed enums?
**A:** Rust-idiomatic typed enums, staged. Model the skeleton as `Component`, `DataSource`, `ValueOrParamRef<T>`, `Mark { kind: MarkKind, data: Option<PlotFrom>, options: MarkOptions }`, but keep `MarkOptions` as a flat `IndexMap<String, ValueOrParamRef<SpecValue>>` initially. Upgrade specific marks to dedicated typed option structs as individual mark-card work lands them. Brief §3.1 says "mirroring, not porting" — exhaustive enums match that, but exhaustively typing 50 mark variants upfront traps us in work downstream cards will redo.

### Q2: Schema authority and version handling
**Q:** Is the canonical schema the upstream Mosaic JSON Schema (validated at runtime via the `jsonschema` crate), the Rust types themselves (serde as the schema check), or a hybrid where our types generate a schema cross-checked against upstream in tests?
**A:** Rust types authoritative for v1. Version-pin to Mosaic spec 0.24.x in a constant, surface it in error output. Serde's `#[serde(deny_unknown_fields)]` scoped to top-level heads (`meta`, `config`, `plotDefaults`); permissive on options bags. Evolve to the hybrid (with `schemars`-generated schema cross-checked against upstream in tests) once five or more marks are implemented. JSON Schema cannot express Mosaic's polymorphic sugar — `DataNode.js:39-43` shows string → `{type:'table', query}` coercions that aren't expressible in standard JSON Schema, so schema-first validation gives worst-of-both.

### Q3: YAML and JSON ingestion path
**Q:** Single serde-derived AST fed by two deserialisers, YAML→`serde_json::Value`→AST, or two parsers with a shared post-processing stage?
**A:** Single serde-derived AST with two deserialisers. Entry point `parse_spec(source: &str, format: Format) -> Result<Spec, ParseError>`; also accept a path-based convenience that sniffs format from extension. Format parity is free at the AST level; diagnostic-location parity is a separate concern (Q4). Mosaic's own test suite round-trips the same AST through both YAML and JSON forms (`packages/vgplot/spec/test/spec.test.js`).

### Q4: Diagnostic model
**Q:** `thiserror` enum with optional source spans, `miette`-powered rich diagnostics from day one, or freeform `anyhow` context chains?
**A:** `thiserror` enum with optional `SourceSpan` now; a `miette`-powered rendering layer comes later as an app-facing concern. `ParseError` carries ~8 variants (unknown mark, unknown component key, unknown input, malformed data def, malformed param def, unresolved `$param`/`$selection` ref in strict contexts, schema violation, IO). `span: Option<SourceSpan>` captured from `serde_yaml::Error::location()` / `serde_json::Error::line/column` where available; `None` for semantic errors detected after deserialisation. Later polish work can add a span-preserving deserialisation pass for semantic spans. Keeps the parser library-facing; the CLI or GPUI error overlay wraps in `miette::Report` downstream.

### Q5: `$param` and `$selection` reference representation
**Q:** Strings verbatim (defer tokenising to the query generator), eagerly parse all forms into structured nodes, or hybrid — structural refs lifted eagerly with SQL-expression interiors staying as opaque strings inside `ExpressionNode { spans, params }`?
**A:** Hybrid. At field-value positions (`filterBy`, `as`, `x`, `fill`, mark option slots) lift string `"$foo"` and object `{param: "foo"}` to `ParamRef("foo")` at parse time — matches Mosaic's `maybeParam` (`parse-spec.js:121-126`). Eagerly tokenise SQL expressions into `ExpressionNode { spans: Vec<String>, params: Vec<ParamRef> }` because Mosaic does (`ExpressionNode.js`) and the tokeniser lives naturally in the parser. Within a span, keep SQL as an opaque `String` — the query generator owns SQL syntax. The coordinator (future card) then walks the AST once to build its subscription graph.

### Q6: Unknown names vs unknown fields — the vocabulary contract (Option Z resolution)
**Q:** Strict `deny_unknown_fields` everywhere, permissive on options bags + error on unknown components/marks/inputs (Mosaic-matching), or warn-on-unknown returning `(Spec, Vec<ParseWarning>)`? **Cross-card note:** this decision contradicted card 0002's D3 requirement that the parser accept the full Mosaic vocabulary so the preflight `SupportReport` can enumerate unsupported features. Resolved together.
**A:** **Option Z — registry-backed vocabulary with implementation status.**

Brightfield maintains a vocabulary enum covering Mosaic's full mark/interactor/input/component set; each entry is flagged `Implemented | Planned | Unimplemented`. Parsing is total per card 0002's D6.1 constraint.

- **Unknown** names (not in the enum at all): hard `ParseError`. Catches typos; catches Mosaic additions we haven't ingested.
- **Known-but-unimplemented** names: parse successfully into an AST stub node carrying its identity and options. Emits a `ParseWarning`. The preflight `SupportReport` (card 0002's concern) consumes these warnings to report unsupported features without failing the parse.
- **Option keys** inside a mark/input/plot attribute bag: warn-on-unknown (accept silently but collect in `ParseWarning`).
- **Top-level heads** (`meta`, `config`, `plotDefaults`): `deny_unknown_fields` (strict — these are closed vocabularies we define).

Return type is `ParseOutput { spec: Spec, warnings: Vec<ParseWarning> }`. Warnings carry structured kind, location span where available, and a message.

---

## Summary

### Goal
A Mosaic YAML or JSON spec parses to a typed Rust AST that the query engine, coordinator, and renderer can drive. The parser is total — every valid Mosaic spec produces an AST, even where marks/interactors/inputs are not yet implemented in brightfield. Malformed specs fail with actionable, located diagnostics. Reactive wiring (`$param`, `filterBy: $selection`) is lifted into structured AST nodes at parse time.

### Constraints
- AST is Rust-idiomatic typed enums with staged typing: skeleton now, per-mark typed option upgrades as each mark card lands.
- AST node taxonomy must cover Mosaic's full component set (stub-and-carry for unimplemented marks — imposed by card 0002 D6.1).
- AST round-trips through serialisation: `parse(spec) → AST → serialise → parse → AST'` with `AST == AST'` (imposed by card 0002 D1 layer 1).
- AST nodes carry source location spans where the underlying deserialiser provides them; span-less errors are acceptable for semantic errors in v1.
- AST nodes are walkable/enumerable and carry stable identities (mark kind, channel name, interactor type, input type) the conformance runner and deviation registry can reference (imposed by card 0002 D6.3, D6.4).
- Mosaic spec version pinned to 0.24.x for v1; version constant surfaced in parser output.
- `$param` / `$selection` structural references lifted at parse time into `ParamRef("name")`; SQL expressions tokenised into `ExpressionNode { spans, params }` with span interiors opaque.
- `deny_unknown_fields` on top-level heads (`meta`, `config`, `plotDefaults`); warn-on-unknown on option bags.
- Diagnostics: `thiserror` enum with optional `SourceSpan`; no `miette` coupling in the parser crate.

### Success Criteria
- All 55 reference YAML/JSON specs in `uwdata/mosaic/docs/public/specs/` parse without error (parser totality).
- AST round-trip test passes for the full reference corpus (property: `parse → serialise → parse` is idempotent on AST).
- Unknown mark/component/input names produce a `ParseError` with a name and location.
- Known-but-unimplemented names produce a `ParseWarning` and an AST stub.
- Malformed YAML / malformed JSON / structural spec errors produce a `ParseError` with location span.
- `$param` and `$selection` references are visible as structured AST nodes (no string re-tokenising needed by the coordinator to build its subscription graph).
- A crossfilter spec (`docs/public/specs/yaml/crossfilter.yaml`) parses to an AST that correctly represents the `params: brush: { select: crossfilter }` declaration, the `filterBy: $brush` references, and the `as: $brush` bindings — all as structured `ParamRef` nodes.

### Decisions Surfaced
- **AST representation**: Rust-idiomatic typed enums with staged typing; flat options bag for marks initially. (→ candidate for a decision record.)
- **Schema authority**: Rust types authoritative; pin Mosaic spec 0.24.x; `schemars` cross-check deferred until mark coverage stabilises.
- **Ingestion**: Single serde-derived AST, two deserialisers (`serde_yaml`, `serde_json`).
- **Diagnostics**: `thiserror` enum + optional `SourceSpan`; no `miette` in the parser crate.
- **Param refs**: Structural refs lifted eagerly; SQL expressions tokenised into `ExpressionNode`; span interiors opaque.
- **Vocabulary (Option Z)**: Registry-backed enum with `Implemented | Planned | Unimplemented` status. Unknown → error; unimplemented → warning + stub. (→ candidate for a decision record; this resolves the cross-card tension with 0002-D3.)

### Open Questions
- `$schema` URL handling at load time — advisory only, or enforced matches against pinned version? (Leaning advisory; revisit under the schema-authority decision.)
- Warning propagation to user — CLI prints, GPUI overlay surfaces, or both? (Renderer concern, out of scope for this card.)
- Inline JSON data (`DataJSONObjects`) — hold rows in AST or stream to DuckDB? (Query-engine concern, out of scope.)
