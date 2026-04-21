# Decision Pack — Card 0001: Mosaic Spec-Driven Visualisation

Foundational contract: a Rust parser that turns Mosaic-compatible YAML/JSON into an AST the coordinator and renderer can drive. Greenfield repo; no existing code constrains us. Every decision below locks in downstream cards (query generator, coordinator, renderer, error reporting).

Evidence citations use repo-relative paths. Mosaic refs are to `/Users/hugh/github/uwdata/mosaic/` (the package is `@uwdata/mosaic-spec`, actually located at `packages/vgplot/spec/`, not `packages/spec/` as the prompt suggested).

---

## Decision 1 — AST representation strategy

**Context.** Mosaic's parser produces an OO tree of `ASTNode` subclasses (`SpecNode`, `PlotMarkNode`, `DataNode`, `ParamRefNode`, ...) with `instantiate()` / `codegen()` / `toJSON()` methods (`packages/vgplot/spec/src/ast/*.js`). Brightfield does not need `codegen` (no ESM output) or `instantiate` (no live web API). We need a tree that the query generator and renderer can pattern-match on. The shape we pick is the interface every downstream card reads against.

**Options.**

- **A. Serde `Value` pass-through.** Deserialise YAML/JSON to `serde_yaml::Value` / `serde_json::Value`, pass unchanged to query generator. Zero schema code.
- **B. 1:1 port of Mosaic's AST classes.** Rust enums/structs mirroring `SpecNode`, `PlotNode`, `PlotMarkNode`, `DataNode` variants, `ParamRefNode`, `ExpressionNode`, etc. Same names, same fields.
- **C. Rust-idiomatic typed AST.** Strongly-typed enums (`Component::Plot | HConcat | VConcat | Input | Legend | Mark`), `DataSource` as sum type, `ValueOrParamRef<T>` for every `$param`-able slot, Mark options as one variant per mark kind. Not a mechanical port — a redesign that uses Rust's type system.

**Trade-offs.**

- A. Cheap to build; defers all validation to query-generator time. But forces every downstream card to re-traverse raw maps and re-check shapes, and loses the parse-time error locus required by AC "Malformed specs surface actionable diagnostics". Moves the problem rather than solving it.
- B. Matches Mosaic's parser tests and documentation 1:1, making cross-reference easy. But OO AST nodes with `instantiate/codegen` methods translate awkwardly to Rust (dyn trait objects or a mega-enum) and carry JS-idiomatic concerns (code generation to ESM) we don't need. `OptionsNode` as a generic bag defers typing forever.
- C. Uses Rust's strengths — exhaustive `match` catches unhandled mark types at compile time, `ValueOrParamRef` makes reactive slots a type-system concern. Higher upfront effort: every mark kind's option set must be modelled. Diverges from Mosaic's naming, so the Mosaic source becomes reference rather than ground truth. `PlotMark` in Mosaic is a union of ~50 mark types (`packages/vgplot/spec/src/spec/PlotMark.ts`) — full coverage is large.

**Recommendation: C, with a staged rollout.**

Model the skeleton as typed enums now (`Component`, `DataSource`, `ValueOrParamRef<T>`, `Mark { kind: MarkKind, data: Option<PlotFrom>, options: MarkOptions }`), but keep `MarkOptions` as a flat `IndexMap<String, ValueOrParamRef<SpecValue>>` initially — exhaustively typing 50 mark variants duplicates work the query generator and renderer already do. Brief §7 calls for a vertical slice (lineY first); the typed skeleton + flat options bag is the minimum that supports that slice without trapping us in serde pass-through debt. Upgrade specific marks to dedicated structs as the renderer lands them (card boundary: one mark-type card = one typed-options upgrade).

Evidence: Brief §3.1 names the output as "A Rust AST mirroring Mosaic's `parseSpec()` output structure" — *mirroring*, not *porting*. Mosaic's `PlotMarkNode` (`packages/vgplot/spec/src/ast/PlotMarkNode.js`) itself stores options as a flat `OptionsNode` bag, so our skeleton is faithful.

---

## Decision 2 — Schema authority and version handling

**Context.** Mosaic's canonical schema is TypeScript (`packages/vgplot/spec/src/spec/*.ts`). They generate a JSON Schema at build time via `ts-json-schema-generator` targeting the `Spec` type (see `package.json` → `schema` script: `ts-json-schema-generator -f tsconfig.json -p src/spec/Spec.ts -t Spec ... > dist/mosaic-schema.json`). Published as `https://uwdata.github.io/mosaic/schema/latest.json` and referenced from specs via `$schema`. Spec currently at v0.24.2. We need to decide what we treat as authoritative and how we handle drift.

**Options.**

- **A. Pin to upstream JSON Schema, validate via `jsonschema` crate.** Check into the repo, validate every input against it before AST construction. Re-vendor on Mosaic release bumps.
- **B. Rust types are authoritative.** Maintain structural parity with Mosaic's TS by periodic manual sync; serde's deserialiser becomes the schema check. Upstream JSON Schema is documentation, not a runtime dependency.
- **C. Hybrid — structural parity in Rust types + generate our own JSON Schema from them (via `schemars`) + use upstream JSON Schema as a cross-check in tests.** Our types are the runtime contract; upstream schema is a golden fixture.

**Trade-offs.**

- A. Cheapest to keep in sync — any Mosaic change ripples in via schema bump. But JSON Schema cannot express Mosaic's polymorphic sugar (a `data` value may be a plain SQL string, an array of objects, or a typed object — see `packages/vgplot/spec/src/ast/DataNode.js` lines 39–43 where `resolveDataSpec` does this coercion in code, not schema). Also binds our error messages to `jsonschema`'s phrasing.
- B. Full control over parse errors and ergonomics. But drift is inevitable — Mosaic ships a new mark and our parser silently rejects it, or worse, mis-parses it. The brief already flags compatibility as a risk (§6 "Spec compatibility"). Needs a manual review cadence.
- C. Runtime is as ergonomic as B (serde-driven); cross-check catches drift. Cost: `schemars` derive on every spec type, plus a test that diffs our generated schema against upstream's. Upstream schema is structural only — won't catch sugar like `data: "SELECT ..."` regardless.

**Recommendation: B for v1, evolve to C when mark-type coverage stabilises.**

JSON Schema can't express enough of Mosaic's shape to be the runtime gate (sugar conversions, `$param` string form, SQL expression objects). The `jsonschema` crate route (A) gives us worst-of-both: we'd still need serde to build the AST, plus schema errors whose phrasing we don't own. For v1 — a vertical slice per brief §7 — serde with `#[serde(deny_unknown_fields)]` scoped to top-level heads (`meta`, `config`, `plotDefaults`) and `#[serde(flatten)]` / permissive options bags elsewhere is enough. Lock Mosaic spec version to `0.24.x` in a constant, surface it in error output. Revisit once five or more marks are implemented — at that point the schemars cross-check pays for itself.

Evidence: `packages/vgplot/spec/src/ast/DataNode.js:39-43` shows string → `{type:'table', query}` and array → `{type:'json', data}` coercions that aren't expressible in standard JSON Schema. `$schema` in `SpecHead` (`packages/vgplot/spec/src/spec/Spec.ts:42`) is advisory — Mosaic itself doesn't validate against it at runtime.

---

## Decision 3 — YAML and JSON ingestion path

**Context.** AC "YAML and JSON are interchangeable spec formats" demands parity. Mosaic inherits this free because both deserialise to JS objects. In Rust, YAML and JSON have distinct deserialisers (`serde_yaml`, `serde_json`) and distinct source-location semantics.

**Options.**

- **A. Single serde-derived AST, two deserialisers.** The AST types `#[derive(Deserialize)]`; YAML and JSON paths both feed into the same types. No intermediate.
- **B. Deserialise both to `serde_json::Value`, then parse from that.** One AST construction path, fed by either format via a shim that converts YAML → JSON `Value`.
- **C. Two parsers with a shared post-processing stage.** Separate deserialisation per format, then a common pass that resolves `$param` strings, coerces data sugar, and builds the typed AST.

**Trade-offs.**

- A. Least code, idiomatic serde. Downside: source-location spans differ (serde_yaml can surface line/col via `Error::location`; serde_json does too via `Error::line`/`column`). Parity of the *AST* is free; parity of the *diagnostics* needs careful handling (Decision 4).
- B. Uniform post-deserialise path, simple to reason about. But we lose YAML's source positions at the conversion step — the `Value` tree has no span info. Unacceptable for any future "underline this line" diagnostic.
- C. Maximum flexibility (can special-case YAML anchors, JSON trailing commas, etc.). But double maintenance; the 55 YAML / 55 JSON example specs (`docs/public/specs/{yaml,json}/`) already prove parity isn't the hard part — the AST is.

**Recommendation: A.**

serde deserialisers into the same `Spec` types is the standard Rust idiom and the thing both Mosaic's tests and the brief §4 tech choices (`serde + serde_yaml`, `serde + serde_json`) are pointing at. Use a `Format` enum at the entry point (`parse_spec(source: &str, format: Format) -> Result<Spec, ParseError>`) or sniff from extension. Address source-location parity as a diagnostic concern (Decision 4), not a deserialisation-architecture concern.

Evidence: `docs/public/specs/yaml/line.yaml` vs `docs/public/specs/json/line.json` are byte-different but AST-identical; Mosaic's own parser tests (`packages/vgplot/spec/test/spec.test.js`) round-trip the same AST through both. No structural post-processing differs between formats in Mosaic.

---

## Decision 4 — Diagnostic model

**Context.** AC 5: "brightfield fails with a diagnostic that names the problem and locates it in the spec, rather than crashing or rendering silently broken output". Mosaic's parser throws `Object.assign(Error(message), { data })` — a message and an unhelpful reference to the offending sub-spec object (`packages/vgplot/spec/src/util.js:45-47`). No line/column, no file path, no error codes. We must do better on day one because Rust has no stack traces to lean on and because spec authors won't be looking at our source.

**Options.**

- **A. `thiserror` enum with typed variants + optional `SourceSpan`.** `ParseError::UnknownMark { name, span }`, `ParseError::UnresolvedDataRef { name, span }`, etc. Span is `Option<Range<usize>>` because not every error has one.
- **B. `miette`-powered diagnostics from the start.** `#[derive(Diagnostic)]`, labelled spans, help messages, colourised rendering built in. One `ParseError` enum, rich by default.
- **C. Freeform `anyhow::Error` with context chaining.** `.context("parsing mark")` → `.context("parsing plot component")` → ... Cheap, message-only, no structured variants.

**Trade-offs.**

- A. Structured errors are programmatically matchable (tests can assert `ParseError::UnknownMark`), and a later card can add a `miette` renderer on top without refactoring call sites. Spans need threading through — serde_yaml and serde_json report byte offsets differently, requiring a small wrapper. Medium cost.
- B. Best UX out of the gate; matches the "actionable diagnostics" AC ambitiously. But couples the AST to `miette`'s derive and forces every error site to know its span — slower to write, harder to produce errors from deep in query-generation time (which is downstream of card 0001). Library-vs-app confusion: `miette` is app-facing.
- C. Fast to write, terrible to consume. Tests can only grep message substrings; users get a `Caused by:` chain. Fails the AC intent.

**Recommendation: A now, miette layer later.**

Define `ParseError` as a `thiserror` enum with ~8 variants covering the shapes the AST parser can actually produce (unknown mark, unknown component key, unknown input, malformed data def, malformed param def, unresolved `$param`/`$selection` ref in strict contexts, schema violation, IO error). Carry `span: Option<SourceSpan>` (custom type holding byte range + line/col) captured from serde errors where available; leave `None` where the error is semantic rather than syntactic. Downstream cards can wrap this in `miette::Report` for the CLI or GPUI error overlay without touching the parser.

Span capture note: `serde_yaml::Error::location()` returns line+column; `serde_json::Error::line()`/`.column()` returns the same. For semantic errors detected after deserialisation (e.g. unknown mark name), we won't have native spans. Two acceptable fallbacks: (i) no span, point at the whole file; (ii) second pass with a span-preserving deserialiser like `serde_yaml::with::singleton_map` or the `serde_spanned` crate. Defer (ii) to a later polish card.

Evidence: AC 5's intent is stronger than Mosaic's current throw-message behaviour, which is explicitly the bar we're raising above ("rather than crashing"). Mosaic's `error()` helper (`util.js:45`) attaches `data` — structured but spanless — confirming spans are new work regardless of approach.

---

## Decision 5 — `$param` and `$selection` reference representation

**Context.** Mosaic allows param references in several syntactic forms:
1. String shorthand: `"$brush"` (prefix `$`).
2. Object form: `{ param: "brush" }`.
3. Embedded in SQL expressions: `{ sql: "delay > $threshold" }` or column form `$$col`.
4. As the value of `filterBy: $brush` / `as: $brush`.

The `paramRef()` util (`packages/vgplot/spec/src/util.js:1-14`) normalises (1)/(2); `parseExpression` (`packages/vgplot/spec/src/ast/ExpressionNode.js:5-25`) tokenises (3). The coordinator, when it receives the AST, must know which fields are reactive. Where does the parsing of these refs happen, and how do they appear in the AST?

**Options.**

- **A. Strings verbatim; defer parsing to the query generator.** Store `"$brush"` as a string in the AST; query generator regex-scans at codegen time.
- **B. Parse all forms eagerly into structured nodes.** Every spec field that can accept a param is typed `ValueOrParamRef<T>` (an enum: `Literal(T)` or `ParamRef(String)` or `ColumnParamRef(String)`). SQL expressions parsed up-front into `(spans, params)` pairs matching Mosaic's `ExpressionNode`.
- **C. Hybrid — structured nodes for top-level slots, strings for SQL expression interiors.** `filterBy` and `as` become `ParamRef`; the SQL inside `{sql: ...}` stays as an opaque string until the query generator tokenises it.

**Trade-offs.**

- A. Simple parse, but every consumer reinvents the tokeniser. The `$` vs `$$` distinction (column ref vs param ref) and escape handling (quoted strings inside SQL expressions — see `tokenRegExp` at `ExpressionNode.js:5`) is real work to duplicate. Also means the coordinator can't build its reactive dependency graph from the AST alone.
- B. AST fully expresses reactive wiring. Coordinator can walk the AST and register subscriptions without re-tokenising. Cost: every value slot on every mark option becomes `ValueOrParamRef<SpecValue>` — verbose, and widens the options-bag type.
- C. Natural boundary — `$param` references as *values* are structural (they mean "substitute"), while `$param` references *inside a SQL string* are part of a larger tokenised expression. Matches Mosaic's split: `util.paramRef` handles structural refs, `parseExpression` handles SQL.

**Recommendation: C.**

At field-value position (`filterBy`, `as`, `x`, `fill`, and every option slot), lift string `"$foo"` and object `{param: "foo"}` to `ParamRef("foo")` at parse time — this is exactly what Mosaic's `maybeParam` does (`parse-spec.js:121-126`). Also eagerly tokenise SQL expressions into `ExpressionNode { spans: Vec<String>, params: Vec<ParamRef> }` because Mosaic does (`ExpressionNode.js`) and the tokeniser lives naturally in the parser, not the query generator. *Within* a span, keep the SQL as an opaque `String` — the query generator owns SQL syntax. The coordinator (card 0003 territory) then walks the AST once to build its subscription graph, which is the cheapest way to meet the "reactive wiring is declared, not coded" AC.

Evidence: Crossfilter spec (`docs/public/specs/yaml/crossfilter.yaml:4`) `{ select: crossfilter }` and `filterBy: $brush` / `as: $brush` show both structural forms in a single 35-line file — the canonical target. Mosaic's own parser resolves these at parse time, not query time.

---

## Decision 6 — Unknown fields and forward compatibility

**Context.** Mosaic evolves — new marks, new input types, new encoding channels land in minor versions. Mosaic's own parser errors on unknown components (`parse-spec.js:110-112` throws "Invalid specification."), unknown marks (`PlotMarkNode.js:19-21` "Unrecognized mark type"), unknown inputs. But it *silently accepts* unknown keys on options bags via the `...options` rest pattern (`PlotMarkNode.js:18`). We have to choose a stance before the first mark lands, because it's baked into serde derives.

**Options.**

- **A. Strict — `deny_unknown_fields` everywhere.** Any unknown key is a hard error.
- **B. Permissive — accept unknown fields silently on options bags, error on unknown components/marks/inputs.** Matches Mosaic.
- **C. Warn-on-unknown — accept but surface a `ParseWarning` collected alongside errors.** Returned as `(Spec, Vec<ParseWarning>)`.

**Trade-offs.**

- A. Catches typos (`fil: steelblue` instead of `fill:`) early — cheap quality win. But breaks every time Mosaic adds a mark option before we upgrade; specs authored against newer Mosaic versions won't load at all. Hostile to the brief's compat goal (§3.1 "Specs authored for Mosaic's web environment should work unchanged").
- B. Maximum compat — specs survive Mosaic version drift. Typos become silent render bugs ("why isn't my fill working?") — precisely the kind of silent failure AC 5 is meant to prevent.
- C. Catches typos without breaking forward compat. Diagnostic output gets busier; tests need to assert warning sets, not just errors. Small extra cost to thread warnings through the parser.

**Recommendation: C.**

Strict components/marks/inputs (errors — these are closed vocabularies we implement one by one, and an unknown one is a genuine unsupported-feature signal worth surfacing loudly). Warn-on-unknown for option keys on mark/input/plot-attribute bags. Return `ParseOutput { spec: Spec, warnings: Vec<ParseWarning> }`. Warnings include the unknown key and a "known keys for mark `lineY` are: ..." hint when feasible. This threads the needle between AC 5's "actionable diagnostics" and brief §3.1's compat promise.

Evidence: Brief §6 "Spec compatibility" acknowledges "edge cases will emerge" — the design already expects deviation. Mosaic's own implementation (`PlotMarkNode.js:17-31`) validates the mark *name* against a registry but accepts arbitrary options — the pattern we're formalising with warnings.

---

## Summary matrix

```
| #  | Decision                     | Recommendation                                                                 |
|----|------------------------------|--------------------------------------------------------------------------------|
| 1  | AST representation           | Rust-idiomatic typed enums; flat option bags for mark options initially        |
| 2  | Schema authority             | Rust types authoritative; version-pin to Mosaic 0.24.x; cross-check later      |
| 3  | YAML/JSON ingestion          | Single serde-derived AST, two deserialisers                                    |
| 4  | Diagnostic model             | `thiserror` enum with optional span now; `miette` rendering layer later        |
| 5  | Param/selection refs         | Eager structural lift; tokenise SQL expressions; strings inside span interiors |
| 6  | Unknown fields               | Strict on component/mark/input names; warn on option-bag keys                  |
```

Open questions for review gate (flagged, not answered):

- Do we want `$schema` URL checking at load time, or purely advisory? (Leaning advisory; tied to Decision 2.)
- When a warning is produced, does the GPUI shell surface it, or is it only CLI-visible? (Renderer concern, out of scope for card 0001.)
- For inline JSON data (`DataJSONObjects`), do we hold the rows in the AST or stream them straight to DuckDB? (Query-engine concern — card 0002.)
