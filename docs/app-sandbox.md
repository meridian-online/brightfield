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
| relaunch-child-bm | after the relaunch, the helper and its child, handed a plain bookmark, read the folder | read |
| relaunch-child-nobm | after the relaunch, the helper and its child, handed **nothing**, read the folder | read (measured 2026-11-26) |
| relaunch-child-control | after the relaunch, the same helper reads the ungranted sibling | refused (measured 2026-11-26) |
| relaunch-duckdb-nobm | after the relaunch, the inherit-signed DuckDB CLI, handed nothing, reads the file | read (measured 2026-11-26) |
| relaunch-duckdb-control | after the relaunch, the same CLI reads the ungranted sibling | refused (measured 2026-11-26) |

What follows from the rows:

1. **A child inherits the folders the analyst granted after launch.** On macOS 27.2 a sandboxed app's children, and their children, get the grants the app holds when it starts them, whether those grants came from an open-documents event or from a resolved security-scoped bookmark, and with no bookmark handed down. Apple's archived Entitlement Key Reference says the opposite, that a child inherits "only the static rights defined in the main app's entitlements file". On this system it does not hold.
2. **DuckDB's own CLI runs as a sandboxed child, and within the grants.** It reads and writes the granted folder and is refused the rest, whether it keeps DuckDB's signature or is re-signed as an `inherit` helper. So the Store build can ship DuckDB's own CLI, rather than a smaller program standing in for it.
3. **Programs outside the bundle do not start, even from a granted folder.** This is Apple's documented rule, and it is what the managed edition's restriction on command steps rests on. `/bin/sh` and the system's own tools do start, and they run sandboxed.
4. **`open -a` grants what it opens.** So does a Finder open, which takes the same route. A Terminal function that calls `open -a Brightfield` is a working way in for the managed edition.
5. **A security-scoped bookmark survives a relaunch.** It was not stale, and the children it reached after the relaunch read the folder too, with nothing handed to them. This is the path a Protocol reopened from Recents and then run takes: the app resolves the bookmark at launch, and arc and DuckDB's CLI, started with no bookmark, read the folder and are refused its ungranted sibling.

## Signing the bundled DuckDB extensions

Unsandboxed, with the pinned CLI, `excel` was installed into a scratch directory and copied twice:

| Copy | `codesign --force --sign -` | `LOAD` with DuckDB's signature check on |
| --- | --- | --- |
| as installed | not run | loaded |
| re-signed | refused: "main executable failed strict validation"; `--no-strict` gives "internal error in Code Signing subsystem". The file is unchanged and keeps only its linker signature | loaded |

A control settles why. A plain dynamic library signed ad-hoc verifies under `codesign --verify --strict`, and the same library with 512 bytes appended fails it with "main executable failed strict validation". Anything after the code signature breaks it, whoever signed it.

Apple's `codesign` cannot sign a DuckDB extension, because DuckDB appends its metadata and signature block after the Mach-O image. The extension still loads, but it stays ad-hoc. The Mac App Store requires every piece of code in a submitted bundle to be signed with the developer's distribution identity, so shipping loadable extension files is blocked as they are built today. The signed download meets the same problem with FineType's extension under notarisation.

## Baseline: today's Brightfield, sandboxed

`scripts/sandbox-measure.sh baseline` wraps `brightfield-shot` in `.app` variants that differ only in their entitlements. `brightfield-shot` boots the real shell and renders offscreen. Each variant opens the edgar_gleif Protocol's `arcform.yaml`, and the script records the detector line on stderr, the exit status and the picture's hash. The temporary-exception entitlements stand in for a panel grant, as the v0.1.2 measurement did: a trailing `/` grants the folder, and its absence grants the file alone.

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
- **It is still drawn without the model, and still exits 0.** A grant on the file alone gives the same picture, byte for byte, as `models/` at mode `000`, at exit 0. Nothing in the window said the Protocol was incomplete beyond the chip's own label. A banner in the window and a non-zero headless exit were needed, and the package measured below has both.
- **A grant on the folder is indistinguishable from no sandbox,** read-only or read-write, as in v0.1.2.
- **Writing needs a read-write grant, and a refused write is loud,** as in v0.1.2.

The run used `brightfield-shot` rather than the window, because the window's first launch offers to make Brightfield the default app. In a measurement run that offer reached macOS's own dialog. The script now answers the offer in each case's preferences, and the headless binary opens no window at all.

## The Store package, measured

On 2026-11-26, macOS 27.2, arm64, the ad-hoc Mac App Store package that `scripts/package-store.sh` builds was measured with `scripts/sandbox-measure.sh baseline --package dist/store/Brightfield.app dist/store/brightfield-shot` and `scripts/sandbox-measure.sh premises dist/store/Brightfield.app`. Each baseline variant carries the package's `Contents/Frameworks`, `Contents/Helpers` and `Contents/Resources` around the managed edition's `brightfield-shot`, so it runs on Meridian's DuckDB library with its extensions compiled in. The probe ran the package's own DuckDB CLI with that library in place of the release CLI. Every probe row came out as in the table above.

| Case | Exit | Detector |
| --- | --- | --- |
| no sandbox | 0 | `protocol edgar_gleif (23 collapsed / 34 full nodes, …)` |
| no sandbox, `models/` at mode `000` | 3 | `degraded step build_sec_entities: unreadable: …/models/sec_entities.sql: Permission denied (os error 13) — the read was refused. …` |
| sandboxed, nothing granted | 1 | `error: read …/arcform.yaml: Operation not permitted (os error 1)` |
| sandboxed, **the file alone** granted | 3 | `… unreadable: …/models/sec_entities.sql: Operation not permitted (os error 1) — the read was refused. …` |
| sandboxed, the folder granted read-only | 0 | the protocol line |
| sandboxed, the folder granted read-write | 0 | the protocol line |
| sandboxed, folder granted, `models/` at mode `000` | 3 | `… Permission denied (os error 13) — the read was refused. …` |
| sandboxed, folder read-only, picture written into it | 1 | the protocol line, then the write refused |
| sandboxed, folder read-write, picture written into it | 0 | the protocol line |
| the Store entitlements, verbatim, nothing granted | 1 | `error: read …/arcform.yaml: Operation not permitted (os error 1)` |
| the Store entitlements, a CSV in `~/Downloads` | 0 | the CSV opened and drew |
| the Store entitlements, the same CSV outside Downloads | 1 | `DDL failed for source 'opened': IO Error: Cannot open file …` |

What changed from the baseline above:

- **A grant that does not reach a model now fails the headless run.** The file-alone grant, and `models/` at mode `000`, exit 3 naming the refused model, where they drew the degraded picture at exit 0. The window reports the same refusal in its banner.
- **The Downloads folder needs no grant.** Under `files.downloads.read-write`, which the Store entitlements carry, a CSV in `~/Downloads` opens with nothing granted, and the same file elsewhere is refused. So the managed edition can treat Downloads as readable without a grant and list it in the sources browser, as it does. Adding such a CSV as a source through the window is a hand case below.

## Hand cases

These need the open panel, Finder or the Run control, which a script cannot drive. Each is to be done on a managed build:

- open a CSV in `~/Downloads` and add it as a source, with no grant panel;
- open a Protocol folder from Cmd+O;
- open its `arcform.yaml` alone from Cmd+O;
- open the folder and the file through Finder's Open With;
- save, quit, relaunch and reopen from Recents;
- run;
- propose joins on a folder of tables;
- make Brightfield the default app for CSV;
- install the managed edition beside the full edition under one bundle identifier.

The default-app and shared-identifier cases change the Mac's own settings, so they were left for the TestFlight build, below. That run also covered opening files through the panel, saving, relaunching and running; adding a Downloads CSV as a source, opening `arcform.yaml` alone, Finder's Open With on a folder, and proposing joins are still to be done.

## The first TestFlight build, by hand

On 2026-11-28 the Store package 0.1.9, signed for the Mac App Store and installed through TestFlight, was run by hand on an M1 Pro MacBook Pro (16 GB, macOS 27.2) in a user account that had run the full edition but never the Store edition. The Protocols folder was `~/Documents/Protocols`.

| Case | Result |
| --- | --- |
| First launch | Asked for the Protocols folder once, opening on the existing `~/Documents/Protocols`. |
| A CSV in `~/datasets`, opened through the panel | Opened and drew. |
| A CSV in iCloud Drive (`~/Library/Mobile Documents/com~apple~CloudDocs`) | Made a Protocol in the Protocols folder, ran it, and drew all 891 rows. |
| A kept step, saved | The step's SQL file, `arcform.yaml` and the dashboard file were written into the Protocol's folder. |
| Quit and relaunch | Reopened the last Protocol with no folder prompt. |
| A step's SQL file at mode `000` | The refusal was reported, naming the file and `Permission denied`. The Protocol did not open, and the report was headed as a data file. |
| Run | Ran to success. Every step was skipped as current, and each table was measured. |
| `edgar_gleif` | Opened restricted, drew its 19 steps, and named its first `uv` step as the one this edition can't run. Run was greyed. The version on open-analytics' `main` uses an operator, `dlt`, that this build's arc does not know, and was refused whole; the version before it was used. |
| Brightfield as the default app for CSV | A double-clicked CSV in `~/Downloads` opened as a new Protocol with no folder prompt. A CSV inside a folder holding an `arcform.yaml` opened that folder's Protocol instead, without saying so. |
| Beside the full edition, both `online.meridian.brightfield` | Each kept its own Recents. Finder's Open With listed one Brightfield, and Launch Services opened files in the full edition's copy. |

What it showed:

- **The grants work as designed.** Each file the checks read or wrote, in the Protocols folder, iCloud Drive and Downloads, was reached through a grant or the Downloads entitlement, and the one file made unreadable was reported as refused, though not as the window's banner should: the Protocol did not open, and the report was headed as a data file.
- **The two editions need different bundle identifiers.** With one identifier shared, Finder treats the two copies as one app and an analyst cannot choose which opens a file. The Store edition keeps `online.meridian.brightfield`, which its App Store Connect record is locked to, and the full edition takes its own.
- **Dropping a file on the window aborted the app.** The drop asked for a folder, and the folder panel ran modally while the drag was still being tracked; a callback in that loop panicked where it cannot unwind. The full edition asks for no folder and does not crash.
- **A long open stalls the window.** Opening a 14,132-row CSV's Protocol kept the main thread busy for about 30 seconds, typing the columns and running each mark's query, so macOS showed the pinwheel.
- **A file inside another Protocol's folder opens that Protocol silently.** The note saying so did not appear, so the analyst saw a different dataset from the one they double-clicked.
- **A folder that does not exist is asked for.** A Protocol whose source points at a deleted folder showed the grant panel, opened on `/Volumes`, before failing to read the file.
