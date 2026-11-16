# Brightfield under App Sandbox

What App Sandbox does to Brightfield, measured rather than read from Apple's pages. It is the evidence behind the managed edition, the Mac App Store build. `scripts/sandbox-measure.sh` reproduces every scripted row below.

## Method

The kernel applies App Sandbox from the entitlements in a program's code signature, whoever signed it. So every row here was measured with ad-hoc signatures (`codesign --sign -`), with no Apple enrolment. Bundle identifiers are throwaway (`online.meridian.sandlab.*`), so nothing measured touches the real app's container or its Launch Services registration.

Measured on macOS 27.2, arm64, on 2026-11-17, against `main` at `2e0f3e1f` and the pinned DuckDB CLI, v2.0.0 `d54cbbe3df`.

## Premises: what a sandboxed app's child processes may do

`scripts/sandbox-measure.sh premises` builds a background-only probe app. It is signed with `app-sandbox`, `files.user-selected.read-write`, `files.bookmarks.app-scope` and `network.client`. It holds a helper copy of itself and two copies of the DuckDB CLI. It is handed a folder by `open -a Probe.app <folder>`, which is the open-documents route a Finder double-click takes. A sibling folder that nobody granted is the control.

| Row | What ran | Result |
| --- | --- | --- |
| parent-read | the app reads a file in the folder it was handed | read |
| parent-control | the app reads the ungranted sibling | refused: the sandbox is working |
| child-nobm | a helper signed `app-sandbox` + `inherit`, handed **no bookmark** | read |
| child-control | the same helper reads the ungranted sibling | refused: the child is sandboxed |
| grandchild | that helper's own child, no bookmark | read |
| self-child-nobm | the app's main executable started as its own child, no bookmark | read |
| sh | `/bin/sh -c 'cat <file>'` started by the app | read |
| duckdb-in-bundle | the DuckDB CLI in `Contents/Helpers`, keeping DuckDB's own signature, `read_text` on the file | read |
| duckdb-inherit-signed | the same CLI re-signed as a helper with `app-sandbox` + `inherit` | read |
| duckdb-control, duckdb-inherit-control | either CLI reads the ungranted sibling | refused: `IO Error: Cannot open file "…": Operation not permitted` |
| duckdb-writes-granted | the CLI writes a CSV into the granted folder | written |
| duckdb-in-granted-folder | a copy of the CLI inside the granted folder | not started: the spawn fails as if the file did not exist |
| duckdb-in-ungranted-folder | a copy of the CLI in the ungranted sibling | not started, the same way |
| scoped-saved | a security-scoped bookmark of the folder is saved in the container | saved |
| relaunch-read | the app relaunched with no document resolves the bookmark and starts access | read, not stale |
| relaunch-child-bm | after the relaunch, the helper and its child read the folder | read |

What follows from the rows:

1. **A child inherits the folders the analyst granted after launch.** On macOS 27.2 a sandboxed app's children, and their children, get the grants the app holds when it starts them, whether those grants came from an open-documents event or from a resolved security-scoped bookmark, and with no bookmark handed down. Apple's archived Entitlement Key Reference says the opposite, that a child inherits "only the static rights defined in the main app's entitlements file". On this system it does not hold.
2. **DuckDB's own CLI runs as a sandboxed child, and within the grants.** It reads and writes the granted folder and is refused the rest, whether it keeps DuckDB's signature or is re-signed as an `inherit` helper. So the Store build can ship the pinned CLI itself, and the stand-in in the design is not needed.
3. **Programs outside the bundle do not start, even from a granted folder.** This is Apple's documented rule, and it is what the managed edition's restriction on command steps rests on. `/bin/sh` and the system's own tools do start, and they run sandboxed.
4. **`open -a` grants what it opens.** So does a Finder open, which takes the same route. A Terminal function that calls `open -a Brightfield` is a working way in for the managed edition.
5. **A security-scoped bookmark survives a relaunch.** It was not stale, and the children it reached after the relaunch read the folder too.

## Premise 12: signing the bundled DuckDB extensions

Unsandboxed, with the pinned CLI, `excel` was installed into a scratch directory and copied twice:

| Copy | `codesign --force --sign -` | `LOAD` with DuckDB's signature check on |
| --- | --- | --- |
| as installed | not run | loaded |
| re-signed | refused: "main executable failed strict validation"; `--no-strict` gives "internal error in Code Signing subsystem". The file is unchanged and keeps only its linker signature | loaded |

A control settles why. A plain dynamic library signed ad-hoc verifies under `codesign --verify --strict`, and the same library with 512 bytes appended fails it with "main executable failed strict validation". Anything after the code signature breaks it, whoever signed it.

Apple's `codesign` cannot sign a DuckDB extension, because DuckDB appends its metadata and signature block after the Mach-O image. The extension still loads, but it stays ad-hoc. The Mac App Store requires every piece of code in a submitted bundle to be signed with the developer's distribution identity, so shipping loadable extension files is blocked as they are built today. The signed download meets the same problem with FineType's extension under notarisation.

## Baseline: today's Brightfield, sandboxed

`scripts/sandbox-measure.sh baseline` wraps today's `brightfield-shot` in `.app` variants that differ only in their entitlements. `brightfield-shot` boots the real shell and renders offscreen. Each variant opens the edgar_gleif Protocol's `arcform.yaml`, and the script records the detector line on stderr, the exit status and the picture's hash. The temporary-exception entitlements stand in for a panel grant, as the v0.1.2 measurement did: a trailing `/` grants the folder, and its absence grants the file alone.

| Case | Exit | Picture | Detector |
| --- | --- | --- | --- |
| no sandbox | 0 | `0344b3aa` | `protocol edgar_gleif (23 collapsed / 34 full nodes, …)` |
| no sandbox, `models/` at mode `000` | 0 | `b2aeacd1` | `degraded step build_sec_entities: unreadable: …/models/sec_entities.sql: Permission denied (os error 13) — the read was refused. …` |
| sandboxed, nothing granted | 1 | none | `error: read …/arcform.yaml: Operation not permitted (os error 1)` |
| sandboxed, **the file alone** granted | 0 | `b2aeacd1` | `… unreadable: …/models/sec_entities.sql: Operation not permitted (os error 1) — the read was refused. …` |
| sandboxed, the folder granted read-only | 0 | `0344b3aa` | `protocol edgar_gleif (23 collapsed / 34 full nodes, …)` |
| sandboxed, the folder granted read-write | 0 | `0344b3aa` | the same |
| sandboxed, folder granted, `models/` at mode `000` | 0 | `b2aeacd1` | `… unreadable: … Permission denied (os error 13) …` |
| sandboxed, folder read-only, picture written into it | 1 | none | the protocol line, then the write refused |
| sandboxed, folder read-write, picture written into it | 0 | `0344b3aa` | the protocol line |

What changed since v0.1.2, and what did not:

- **A refused model is now named as refused.** The loader tells a refused read from a missing file, and stderr says "the read was refused", naming the model.
- **It is still drawn without the model, and still exits 0.** A grant on the file alone gives the same picture, byte for byte, as `models/` at mode `000`, at exit 0. Nothing in the window says the Protocol is incomplete beyond the chip's own label. Task 6 of the plan, a banner in the window and a non-zero headless exit, is still needed.
- **A grant on the folder is indistinguishable from no sandbox,** read-only or read-write, as in v0.1.2.
- **Writing needs a read-write grant, and a refused write is loud,** as in v0.1.2.

The run used `brightfield-shot` rather than the window, because the window's first launch offers to make Brightfield the default app. In a measurement run that offer reached macOS's own dialog. The script now answers the offer in each case's preferences, and the headless binary opens no window at all.

## Hand cases

These need the open panel, Finder or the Run control, which a script cannot drive. Each is to be done on a managed build:

- open a Protocol folder from Cmd+O;
- open its `arcform.yaml` alone from Cmd+O;
- open the folder and the file through Finder's Open With;
- save, quit, relaunch and reopen from Recents;
- run;
- propose joins on a folder of tables;
- make Brightfield the default app for CSV;
- install the managed edition beside the full edition under one bundle identifier.

The default-app and shared-identifier cases change the Mac's own settings, so they are left for the TestFlight build rather than run here.
