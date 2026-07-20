# brightfield-spec

Parser for the Mosaic declarative visualisation spec (YAML/JSON → typed
Rust AST). The stable contract that drives brightfield's query engine,
coordinator, and renderer.

## Parser entry points

Two entry points, single `ParseOutput` return shape:

```rust
use brightfield_spec::{parse_spec, parse_spec_path, Format, ParseOutput};

// From source text with an explicit format
let out: ParseOutput = parse_spec(source, Format::Yaml)?;

// From a path — format sniffed from .yaml / .yml / .json extension
let out: ParseOutput = parse_spec_path("spec.yaml")?;
```

Both return `Result<ParseOutput, ParseError>`, where:

```rust
pub struct ParseOutput {
    pub spec: Spec,
    pub warnings: Vec<ParseWarning>,
}
```

Fatal errors (malformed input, unknown vocabulary names, structural
violations) surface as `ParseError`. Non-fatal observations
(known-but-unimplemented names, unknown option keys on lenient heads,
version mismatches) surface as `ParseWarning`.

## ParseWarning variants

- `Unimplemented { name, surface, status }` — a known vocabulary name whose
  implementation status is `Planned` or `Unimplemented`. The parser emits an
  AST stub carrying the same `ImplStatus`, so downstream preflight (a
  `SupportReport`) can walk the tree and list gaps.
- `UnknownOption { path, key }` — an unknown key on a head that accepts
  unknown keys leniently (see [Option Z](#option-z-vocabulary-contract)).
- `VersionMismatch { declared, supported }` — `meta.version` does not share
  major+minor with `SUPPORTED_MOSAIC_MAJOR_MINOR`.

## Option Z vocabulary contract

Every Mosaic 0.24.x mark, interactor, input, legend channel, component, and
selection-resolution name brightfield recognises is enumerated in
`src/vocab.rs` as a sealed enum with an `ImplStatus` (`Implemented`,
`Planned`, `Unimplemented`). The parser's behaviour on names:

| Name's state in the registry | Parser behaviour |
|---|---|
| Absent from the registry | Fatal: `ParseError::UnknownName` |
| Present, `ImplStatus::Implemented` | Parse into the typed AST node |
| Present, `ImplStatus::Planned` | Parse + stub + `ParseWarning::Unimplemented` |
| Present, `ImplStatus::Unimplemented` | Parse + stub + `ParseWarning::Unimplemented` |

This lets a spec that uses planned-but-not-yet-built vocabulary still be
parsed (and the gaps enumerated), while catching typos or truly foreign
names early.

Strictness of option/attribute keys is split between heads:

- **Meta (`meta:`)** — typed accessors for `title`, `description`, `version`.
  Unknown keys are accepted as `ParseWarning::UnknownOption` (see the
  corpus-driven detour in the spec's progress log).
- **Config (`config:`), PlotDefaults (`plotDefaults:`)** — open bags.
  Mosaic's plot-attribute library drives the shape here; brightfield does not
  own the schema.
- **Mark / Interactor / Input / Legend option bags** — open `IndexMap`.
  Values may lift to `ParamRef` at positions on the lift surface.

Strict-context `$param` detection still fires on string-typed fields under
all three heads (`meta`, `config`, `plotDefaults`): a `$name` reference in
those positions is a fatal `StrictContextUnresolvedRef` because downstream
consumers use those values literally.

## Version pin

```rust
pub const SUPPORTED_MOSAIC_VERSION: &str = "0.24.x";
pub const SUPPORTED_MOSAIC_MAJOR_MINOR: (u16, u16) = (0, 24);
```

A spec's `meta.version` is compared against the pinned major+minor. The
parser does not refuse mismatched versions — it emits
`ParseWarning::VersionMismatch` and proceeds on the best-effort principle.
Callers that want strict behaviour can filter warnings and treat
`VersionMismatch` as fatal.

## v1 non-goals

The v1 parser is deliberately narrow:

- **No textual round-trip.** Parse → serialise → parse is AST-idempotent,
  not byte-identical. `ParamRef` canonicalises to the `$name` string form
  on serialisation; `{param: name}` and `{selection: name}` object
  shorthands parse in but do not round-trip out. Expressions preserve
  `$ident` positions but not whitespace verbatim.
- **No per-mark typed option schemas.** Option bags are
  `IndexMap<String, ValueOrParamRef<SpecValue>>`. Per-mark typing
  (e.g. a typed `x-channel` for line marks) is a follow-up card.
- **No `miette` dependency.** `SourceSpan` is a plain public struct; error
  formatting is `thiserror`-based. Downstream app crates may wrap
  `ParseError` in `miette::Report` themselves for terminal rendering.
- **Best-effort spans.** The underlying deserialiser provides spans on
  syntactic errors; spans on semantic errors (unknown names, schema
  violations) are `None` in v1.
- **No preflight support report generator.** `SupportReport` that walks a
  parsed AST and enumerates unimplemented vocabulary is out of scope here.

## Tests

- Unit tests in each module, named `dfspec_acNN_<scenario>` keyed to the
  spec's acceptance criteria.
- `tests/corpus_totality.rs` — corpus gate. Every YAML in
  `vendor/mosaic-specs/yaml/` must parse without `ParseError`.
- `tests/crossfilter.rs` — crossfilter gate. Structural assertions on the
  canonical crossfilter spec.
- `tests/diagnostics.rs` — Six malformed-spec cases each asserting
  the expected `ParseError` variant.

## Vendor corpus

See `vendor/mosaic-specs/README.md` for the vendored corpus, the upstream
commit SHA, and the refresh procedure.
