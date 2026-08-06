# `examples/remote/` — specs whose data is fetched, not shipped

Every spec in `examples/*.yaml` is self-contained: its rows are inline in the
file or generated in SQL, so it opens with no network and no second file.
The specs in **this** directory are the exception, and the directory exists so
that the difference is a fact about a path rather than something you find out by
running one.

A spec here reads an `https://` source through DuckDB's `httpfs` extension.

## What that costs, precisely

**With a connection**, nothing unusual happens: the engine loads `httpfs`,
DuckDB range-reads the remote Parquet, and the marks execute over it like any
other source.

**With no connection, the spec does not open.** DuckDB binds a view over a
remote Parquet *eagerly* — it reads the file's schema at `CREATE VIEW` — so the
failure happens when the document is loaded, not when a mark is drawn. The
engine classifies it and returns a structured error naming the network and the
URL (`EngineError::RemoteSourceFailed`, or `RemoteDisabled` when `httpfs` itself
cannot be obtained). Nothing here degrades into a blank chart or a partial one.

Where that error surfaces depends on how you opened it:

| how it was opened | with no network | held by |
|---|---|---|
| `brightfield examples/remote/<spec>.yaml` | the process exits non-zero, having printed the error | `scripts/verify-airgapped.sh`, run 3, against the packaged artifact inside a network-denying jail |
| the front door's gallery card | an error banner naming the start and the cause; the window stays up and every other start still opens | `MeridianApp::open_start`'s error arm, which raises a notification and returns rather than taking the window down. Read from the code, not from a separate test — what IS tested is that the load fails this way, in `tests/crosswalk_chart.rs` |

The bundled starting point that opens `edgar-gleif-crosswalk.yaml` says
`(over the network)` on its own button for this reason — the disclosure is made
at the click, which is the only place it can be made before the cost is paid.

## Why they are not in `examples/`

Two mechanical reasons, both worth knowing before moving a file up a level:

1. **`crates/brightfield-shell/tests/examples_exercise.rs` composes every flat
   `examples/*.yaml`** to hold the legend law over the whole corpus. It is a
   GPU-free, network-free test, and a spec that fetches would make it neither.
2. **`scripts/package.sh` copies `examples/*.yaml` into the artifact** beside a
   README that calls them self-contained. A packaged spec that needs a
   connection would falsify that sentence for the whole directory.

`examples/live/` is the neighbouring case — specs that need a local data file
this repo does not vendor — and it is kept out of the flat corpus for the same
kind of reason.

## What holds the claims above

`crates/brightfield-shell/tests/crosswalk_chart.rs`. It runs the offline case
hermetically, by denying the engine the `httpfs` extension rather than by
unplugging anything, and asserts the error names the network and the URL. The
network-gated half of that file (`cargo test -- --ignored`) is what checks the
live source still answers.
