# Brightfield's managed edition on the Mac App Store

Status: approved by Hugh, 2026-11-16; revised on 2026-11-17 after the re-measure (`docs/app-sandbox.md`), with his rulings on the two changes it forced. Planned now; built after the signed download ships.

## Intent

Brightfield goes on the Mac App Store so that an analyst can install it from the place most Mac users trust, and so that schools and companies whose IT departments only allow Store apps can use it. Meridian's public launches and its link-building wait for this listing, which is why it is planned now.

The Store requires App Sandbox. Under the sandbox, Brightfield as built today cannot read a Protocol it was not handed as a folder, cannot run a Protocol at all (every run happens in a child process that does not inherit the analyst's grants), and draws a Protocol whose models it was refused as if the models were simply missing. This design makes a sandboxed Brightfield that opens and saves the analyst's own Protocols, keeps them across relaunches, runs them, and says plainly when it was refused something and how to grant it.

### Editions

Hugh sees room for three editions, defined by what they let an analyst do rather than by where they are downloaded:

| Working name | Channel | What it is |
| --- | --- | --- |
| full | Homebrew, the install command and the signed download | Runs everything: command steps, the analyst's own programs, any DuckDB extension. Unsandboxed. |
| managed | Mac App Store | Opens, edits, saves and runs Protocols with arc's own operators and a fixed set of DuckDB extensions shipped inside the app. Runs no program that is not in the app. Suited to education and corporate IT, where arbitrary code may not run. |
| viewer | App Store, iPad first | Views dashboards, queries datasets and works with what a Protocol has produced. Not built here. |

The names are working names. The final names are a separate piece of work; they are not "Pro" or "Advanced", which would collide with Meridian Pro, the web plan. Only the banner wording in this design depends on them.

### The business model in every edition

Running a managed Protocol and querying a managed dataset must be easy and encouraged in every edition, including this one. The one caveat is Hugh's: when it makes sense to develop and run locally, that is what the analyst should do. Meridian grows without displacing the analyst's delight in their own Mac.

## Goals

1. Brightfield builds as a sandboxed app that App Review can accept, from the same code as the full edition, with a different packaging step and an edition switch.
2. The analyst grants a folder once and keeps it: the grant is saved as a security-scoped bookmark, so a Protocol opened before a relaunch opens after it without a prompt.
3. Grants are taken on folders. Picking a Protocol's `arcform.yaml`, or dropping it, opens its folder, and if the folder is not granted, the window asks for it rather than drawing the Protocol without its models.
4. Every grant is read-write, because Brightfield writes into the Protocols it opens.
5. A refused read is reported in the window, naming the path, with a Grant access action that opens the system panel on the refused folder. It is never drawn as an absent model at success.
6. Runs and join proposals work under the sandbox: arc, dovetail and DuckDB's CLI reach the granted folders.
7. A Protocol with a step this edition cannot run opens restricted, and says which step, why, and where the full edition is.
8. Managed Protocols and managed datasets work and are easy to reach: the edition has network access, ships httpfs and ducklake, and does not stand in the way of signing in to a Meridian profile.
9. The four App Store questions are answered below, with the steps only Hugh can take named.

## Non-goals

- The viewer edition, and any iPad build.
- Final edition names.
- Policies an IT department sets beyond what the sandbox enforces (limits on network volume, CPU or GPU use, configuration profiles). That is a separate research question.
- In-app purchase. Meridian Pro stays a web product tied to a profile (Hugh's ruling below).
- Signing in to a Meridian profile from the app. This design only keeps the way open.
- Bundling uv, bun or a Python interpreter. Possible later, if managed-edition analysts ask for scripts; App Review's reading of it is uncertain.
- Carrying preferences, Recents or trust from a full-edition install into the managed edition. The managed edition keeps its own, inside its container.
- Writing a Store launcher onto the analyst's PATH.

## Anti-goals

- A managed edition that draws a Protocol without part of it and looks as if it worked. This is the failure measured on v0.1.2, and it is the reason for this design.
- A first launch that asks a stranger a list of permission questions before they have seen anything. Only the Protocols folder is asked up front; every other grant is asked at the moment it is needed.
- Shipping a run-time extension download in the Store build and hoping review misses it.
- Two code bases. The managed edition is the full edition's code with a capability list switched.

## Constraints

- **App Sandbox, from Apple's pages.** An open or save panel, and a drop on the Dock icon, extend the sandbox to the chosen item; a folder grant extends recursively to everything inside it. Grants do not survive relaunch unless saved as security-scoped bookmarks and reopened with `startAccessingSecurityScopedResource`. Apple gives no entitlement for `~/Documents`; Downloads has one. A sandboxed app "can't run programs in locations outside its app bundle, sandbox container, or app group containers". A child process inherits "only the static rights defined in the main app's entitlements file, not any rights added to your sandbox after launch", and Apple's remedy is to "pass a bookmark to the child process". A helper tool is signed with exactly `com.apple.security.app-sandbox` and `com.apple.security.inherit`; the main app never carries `inherit`. Code is signed inside out.
- **Under the sandbox, the home directory is the container.** Every `~/…` path Brightfield and arc build today moves into `~/Library/Containers/<bundle id>/Data/`: the default Protocols folder, Application Support, arc's `~/.arcform` working databases, history and fetch cache, and DuckDB's `~/.duckdb`.
- **App Review, guidelines 2.4.5(iv) and 2.5.2.** A Mac App Store app "may not download or install … additional code, or resources to add functionality", and may not "download, install, or execute code which introduces or changes features or functionality of the app". Downloading DuckDB extensions at run time falls under this.
- **Every piece of code in a Store bundle is signed with the developer's distribution identity, and a DuckDB extension file cannot be.** DuckDB appends its metadata and signature block after the Mach-O image, and Apple's `codesign` fails anything with bytes after its signature (`docs/app-sandbox.md`, premise 12).
- **A sandboxed app's children inherit the folders the analyst granted.** Measured on macOS 27.2 (`docs/app-sandbox.md`): a helper, its child, the main executable started as a child, `/bin/sh` and DuckDB's own CLI all read and write a granted folder with nothing handed to them, and are refused the rest. Programs outside the bundle do not start, even from a granted folder.
- **arcform runs SQL through an engine's command-line program**, so that one manifest can target other engines. arc calls the DuckDB CLI in at least five forms: `--version`, checked against a table of known DuckDB release commits; `-noheader -list -c <sql>`; `<db> -f <file>` followed by dot-commands (`.bail off`, `.mode list`, `.headers off`, `.once '<path>'`) and a query of `duckdb_extensions()`; and `-no-init -json` and `-no-init -list -noheader` with a script on stdin, for its lineage tokenizer, which calls `sql_tokenize` from DuckDB's autocomplete extension. It sets `DUCKDB_AGENT_MODE=0` on each, and runs the program `ARC_DUCKDB_BIN` names when set. Brightfield's own engine check adds `-batch -noheader -csv -init <file> -c`. This design does not change arcform.
- **Brightfield's existing rulings hold.** Protocols are opened as folders and worked on where they are; new Protocols are made in the Protocols folder, a preference whose default is `~/Documents/Protocols`; a Protocol from a link, an archive or outside the Protocols folder opens restricted until trusted; the command line that opens things in the window is supported on builds that can offer it.
- **Build slots.** The signed download's sessions have first call on the Mac's Rust builds. The re-measure and every build task run only when a slot is free.

## Approaches

How runs reach the analyst's files was the one open choice. A run goes Brightfield → arc → the DuckDB CLI; a join proposal goes Brightfield → dovetail.

- **A. A DuckDB CLI stand-in inside Brightfield**, DuckDB's own shell compiled in, resolving bookmarks handed to it. Chosen on 2026-11-16, on Apple's archived statement that a child inherits only the app's static rights.
- **B. arc gains an in-process DuckDB runner.** Work in a second repo, and an exception to arc's engine-CLI decision.
- **C. Copy the Protocol into the container, run, and copy back.** Rejected: it breaks the ruling that a Protocol is worked on where it lives, and duplicates large sources.
- **D. The real DuckDB CLI as a sandboxed helper (chosen on 2026-11-17).** The re-measure showed that children inherit the analyst's grants, DuckDB's CLI included. So the Store bundle ships DuckDB's own CLI, signed as a helper, and runs behave exactly as in the full edition because it is the same program. Hugh chose it over keeping A. The stated risk: Apple's archived documentation says otherwise, so a later macOS could change the behaviour, and A would then come back as the fix.

How the extensions reach the managed edition was forced by the same measurement, since an extension file cannot be signed:

- **Built into DuckDB (chosen, Hugh, 2026-11-17).** DuckDB and the extensions the edition needs are compiled together from the pinned source, for the app's in-process library and for its CLI, so no extension file ships.
- **Wait for the signed download's answer** to FineType's extension under notarisation. Not chosen: notarisation is looser than App Store review, so its answer may not carry over.
- **Ship without extensions first.** Not chosen: no workbooks, no SQLite, and no managed datasets or DuckLake.

## The design

### 1. The edition switch

One type, the edition, decides what this build may do. It is set at build time (a cargo feature the Store packaging turns on) and read in one place. Each capability below is a question asked of it, so a third edition is another value, not another fork:

| Capability | full | managed |
| --- | --- | --- |
| Run `command:` steps and shell preconditions | yes | no |
| Run arc operators that start a program outside the app | yes | no |
| Install DuckDB extensions from the network | yes | no; load only the bundled set |
| Install the launcher command | yes | no |
| Read paths with no grant | yes (TCC aside) | no; ask for a grant |

### 2. Grants

A grant store in Brightfield's config directory keeps one security-scoped bookmark per granted folder (`com.apple.security.files.user-selected.read-write` and `com.apple.security.files.bookmarks.app-scope`). At launch it resolves each bookmark and starts access; a stale bookmark is re-saved; a bookmark that no longer resolves is dropped and the folder is reported as needing a grant the next time it is used.

Every way a path arrives is checked against the store:

- **The open panel** (Cmd+O, the front door's picker): the chosen items are granted and saved. A chosen `arcform.yaml` is not enough: its folder is what is needed, so the window opens the panel on that folder with a sentence saying why.
- **Finder's Open With and double-click, Dock drops and window drops**: what the system grants is saved. If a file arrives whose Protocol folder or sibling sources are not granted, the window asks for the folder.
- **Recents and the last Protocol reopened at launch**: covered by their saved grant, or reported as needing one, never opened half-read.
- **The Protocols folder**: asked for once at first launch (below), and its grant covers every Protocol made in it.
- **Sources outside the Protocol folder**: a source is read through the grant on a folder that contains it. When none does, adding the source asks for its folder.
- **A lone data file** (a double-click, the default app, a drop): opening it makes its Protocol in the Protocols folder, as every edition does, and the source is read through the grant the system gave on the file, which is saved like any other. Nothing is written beside it. A DuckDB database file is opened read-only; if DuckDB needs its folder to open it (a write-ahead log beside it, say), the window asks for the folder then.
- **Folder pickers** (Move the Protocol into, Make the Protocol in, Use as the Protocols folder): already system panels; their result is saved as a grant.
- **The typed Location field and the sources browser**: these reach paths without a panel. In the managed edition the Location field accepts only a granted folder and otherwise opens the panel on what was typed; the sources browser lists granted folders and Downloads as its places, with a "Grant a folder…" entry that opens the panel.

Writes need no second grant: every grant is read-write.

### 3. The first launch

The first launch of the managed edition asks one thing: where the Protocols folder is. The panel opens at Documents with a folder named Protocols proposed, so the common answer is one click, and the analyst can choose another place. Nothing else is asked until it is needed.

### 4. Refused access, reported

A refused read is reported wherever it happens:

- **A model** the protocol graph classes as unreadable is already labelled on its chip and in the inspector. The window adds a banner naming the refused path and offering Grant access, which opens the panel on the nearest folder to grant. The Protocol is not shown as complete while the banner stands.
- **A source or model refused during a run**, from arc, the DuckDB CLI or dovetail, comes back as a refused-access error naming the path, not as a failed step with the engine's text alone, and raises the same banner.
- **A write refused** (saving, an export, a run's outputs) does the same.

The banner and the exit below apply to both editions, since a refusal can come from macOS's privacy controls in the full edition too; the plan carries the full edition's regression checks for it.

The headless capture path keeps writing its degrade report to stderr, and gains a non-zero exit when a model read was refused, so a check reading the exit code cannot pass a half-drawn Protocol.

### 5. Children reach the grants

Children inherit the folders the analyst granted, so Brightfield passes nothing to them. What it must do is hold the grants open while a child runs: at launch it resolves every saved bookmark and starts access, as section 2 says, and it does not stop access to a folder while a run or a join proposal that uses it is going.

Each child is a helper executable inside the bundle, signed with only `app-sandbox` and `inherit`, as Apple documents. The re-measure found that the main executable started as its own child also works; the helper copy is kept because it is the documented route and App Review checks the bundle against it.

arc's working databases, history and fetch cache, and DuckDB's temporary directory, live inside the container. That needs no setting: under the sandbox the home directory and temporary directory already resolve there.

### 6. DuckDB with its extensions built in

The managed edition's DuckDB, both the library linked into Brightfield and the CLI arc runs, is built by Meridian from the pinned DuckDB release's source, with the extensions the edition needs compiled in statically: autocomplete, excel, sqlite, httpfs, ducklake, spatial and FineType. No `.duckdb_extension` file ships, so nothing in the bundle carries a signature Apple cannot verify.

The CLI arc runs is DuckDB's own shell from that build. `--version` must print what the pinned release prints, including the release commit arc checks.

Brightfield links the same build in place of duckdb-rs's bundled DuckDB in the managed edition. The full edition keeps its present DuckDB and loadable extensions.

FineType is Meridian's own extension and the least proven to build statically. If it cannot be, the edition ships without in-process FineType typing first and says so, rather than block the rest. That fallback goes to Hugh before it is taken.

### 7. Execution parity

Hugh's condition, "as long as there are no execution parity issues", now applies to Meridian's own build of DuckDB rather than to a stand-in:

- Every Protocol Brightfield ships that runs offline, and the agent Protocols the repo already checks, run under the release CLI and under Meridian's build, and their outputs (tables, run records, exit statuses) compare equal, with any difference treated as a failure.
- arc's engine and tokenizer tests, from arcform at the revision Brightfield pins, pass against Meridian's build through `ARC_DUCKDB_BIN`.
- The gate runs on changes to the build, the packaging, and the arcform or DuckDB pin, and before each Store build.

### 8. Steps this edition cannot run

Before a run, and on open, Brightfield asks the edition about each step: a `command:` step, a shell precondition, an operator that starts an outside program, or a DuckDB extension outside the built-in set.

A Protocol with any of them opens restricted, through the restricted mode Brightfield already has for untrusted Protocols: its spine, steps, SQL and last results are shown, and nothing runs. The banner names the first such step and the reason, for example "Step *clean* runs `uv`, which this edition can't run", and links to the full edition on meridian.online. Trust does not lift this restriction; only the full edition does.

Meridian's own FineType CLI is the exception among outside programs: arc's FineType operators run it, and it is Meridian's reviewed code, so the Store bundle ships it as a helper and points arc at it through `FINETYPE_BIN`. Its operators run in the managed edition.

A Protocol made in the managed edition cannot gain such a step without being told at the moment it is added.

### 9. Extensions

The managed edition's extensions are the ones built into its DuckDB (section 6). Automatic install and load are off, and an `INSTALL` or a `LOAD` of anything else is refused with the edition banner.

### 10. Network and Meridian's products

The managed edition carries `com.apple.security.network.client`. Links from meridian.online, archive downloads, `http_fetch` steps, httpfs and ducklake reads of managed datasets work as in the full edition. Managed Protocols and datasets are offered as they are in the full edition. Outside the United States storefront the app shows no button or link to buy Meridian Pro (guideline 3.1.1); it may encourage Meridian Public, signing in and hosted runs for a profile that has Pro. In the United States storefront a link to the web purchase is allowed.

### 11. Opening from Terminal

The managed edition does not read paths from its own command line. `open -a Brightfield <path>` goes through the same macOS route as a Finder double-click, and grants the path, as the re-measure confirmed. The documentation for the managed edition says so, and gives the one-line shell function an analyst can add themselves. The capture harness's command line is unchanged in the full edition.

### 12. Packaging

A Store packaging path beside the existing one:

- Signing for the Store runs in CI, beside the release and signing-rehearsal workflows, with the Store certificates and profile as secrets.
- An entitlements file for the app (`app-sandbox`, `files.user-selected.read-write`, `files.bookmarks.app-scope`, `files.downloads.read-write`, `network.client`) and one for the helper (`app-sandbox`, `inherit`).
- The bundle holds the main executable, the helper, Meridian's build of the DuckDB CLI and FineType's CLI as further helpers, and FineType's model. No extension file ships.
- Signing inside out with the Mac App Distribution certificate and the Mac App Store provisioning profile embedded, then a `.pkg` signed with the Mac Installer Distribution certificate, uploaded to App Store Connect.
- `ITSAppUsesNonExemptEncryption` false: Brightfield uses only the system's HTTPS.
- The same bundle identifier as the full edition, `online.meridian.brightfield`. Apple says the two builds do not share privacy grants, which is expected; whether a shared identifier also makes the two share a container or a Launch Services registration in a way that confuses an analyst with both installed is checked on the first TestFlight build, and a separate identifier is the fallback.

## The re-measure

Run on 2026-11-17 against `main` at `2e0f3e1f`. The method, every result and what each means are in `docs/app-sandbox.md`, and `scripts/sandbox-measure.sh` reproduces the scripted rows. In short:

- Children, DuckDB's own CLI included, inherit the analyst's grants. Section 5 and approach D follow from it.
- `open -a` grants what it opens; a security-scoped bookmark survives a relaunch and its grant reaches the children.
- Programs outside the bundle do not start, even from a granted folder; `/bin/sh` does, sandboxed.
- A DuckDB extension file cannot be code-signed by anyone. Section 6 follows from it.
- Today's build still draws a Protocol granted on its `arcform.yaml` alone exactly as one whose `models/` are unreadable, at exit 0, though stderr now names the refusal. Section 4 stands.

Left for the first TestFlight build, because they change the Mac's own settings: making Brightfield the default app under the sandbox, and installing the managed edition beside the full edition under one bundle identifier. The hand cases through the open panel, Finder and the Run control are part of the proof run.

## The four App Store questions

### App Store Connect

Prerequisite: the Apple developer membership's move to Meridian's company is complete.

Only Hugh, as Account Holder, can:

- accept the latest Apple Developer Program License Agreement once the membership has moved (App Store Connect refuses a new app record until he does);
- add the first Admins.

Since the managed edition is free and sells nothing through Apple, the Paid Apps Agreement, its tax forms and banking are not needed. Apple's wording is that the Paid Apps Agreement is required "to sell your apps on the App Store or offer In-App Purchases", and once accepted it "can't [be undone]". This should be confirmed in App Store Connect when the membership has moved.

An Admin can then register the explicit App ID, create the Mac App Distribution and Mac Installer Distribution certificates and the Mac App Store provisioning profile, and create the app record.

The listing also needs things only Hugh can supply or approve: the privacy policy URL and the App Privacy answers, the age rating, the description and screenshots, and the support URL.

### Meridian Pro and in-app purchase (guideline 3.1)

Hugh's ruling, 2026-11-16: the managed edition does not offer in-app purchase. Pro is a web product, tied to a profile, and the infrastructure it sells runs off the analyst's machine. The app does not sell Pro, does not link to buying it outside the United States storefront, and lets an analyst sign in to a profile bought on the web.

The risk, stated so it is not a surprise at review: guideline 3.1.3(b) allows a multiplatform service's purchases made elsewhere to be used in the app "provided those items are also available as in-app purchases within the app". App Review may raise it when sign-in arrives. It does not bear on the first submission, which has no sign-in.

### Code loaded at run time (guideline 2.5.2)

DuckDB's extension downloads conflict with 2.4.5(iv) and 2.5.2. The managed edition's extensions are compiled into its DuckDB, and it downloads none (design sections 6 and 9). FineType's model already ships in the bundle. Data downloads, such as a Protocol's sources, a managed dataset or an archive from meridian.online, are data, not code, and are unaffected.

### Signing the nested executables

The helpers (Brightfield's executable in its arc and dovetail roles, Meridian's build of the DuckDB CLI, and FineType's CLI) are signed with `app-sandbox` and `inherit` only, with the distribution identity. No extension file ships, because a DuckDB extension cannot be code-signed. Signing runs inside out: helpers first, then the app. Hardened runtime is not required for the Store, and with no loadable libraries library validation can stay on.

## Hugh's rulings in this brainstorm, 2026-11-16

- No channel may cost the analyst capability "is an important principle", and Hugh is "prepared to be pragmatic".
- Meridian Pro: "Pro will be a web product, tied to a profile. I don't think we need to commit to in-app purchases."
- Editions: full through Homebrew, managed through the App Store for environments "where arbitrary code can't run", and a viewer edition for the App Store and iPad. Names are open.
- IT policies beyond the sandbox are deferred to research or a later brainstorm.
- Every edition makes Meridian's managed Protocols and datasets easy and encouraged, while local development and runs stay the default where they make sense: "We want to grow our business but not while displacing customer delight."
- Approach A, "as long as there are no execution parity issues".
- 2026-11-17, after the re-measure: ship DuckDB's real CLI as a sandboxed helper rather than the stand-in, accepting that a later macOS could change the inheritance it relies on.
- 2026-11-17: build the extensions into DuckDB for the managed edition, rather than wait for the signed download's answer or ship without them.
- A Protocol the managed edition cannot run opens restricted and names the step and the full edition ("This looks like the right approach").

## Calls made on Hugh's behalf

- Only the Protocols folder is asked at first launch; every other grant is asked when it is needed.
- The full edition's command steps and shell preconditions are both refused in the managed edition, since a precondition is arbitrary shell too.
- Built-in extensions: autocomplete, excel, sqlite, httpfs, ducklake, spatial, FineType. Spatial is large; it is in because a shipped Protocol loads it.
- The managed edition does not import a full edition's preferences, Recents or trust.
- The same bundle identifier as the full edition, subject to the first TestFlight build.
- The headless path exits non-zero on a refused model read.
- Trust does not lift the edition restriction.

- Brightfield holds grants open while a child uses them, and passes nothing to children.
- If FineType cannot be built into DuckDB, the edition ships without in-process FineType typing first, after Hugh agrees.
- FineType's CLI ships as a helper, so arc's FineType operators run in the managed edition.
- The refused-access banner and the headless exit apply to both editions.
- The plan runs to the milestone's end: an App Store submission and the run that proves it on a Mac that has never had Brightfield. Signing runs in CI.

## Left to the builder

- The grant store's file format, and how stale bookmarks are refreshed.
- The banner wording, within the rule that it names the path or step and the action.
- Where the bundled extensions sit inside the bundle.
- How the restricted check walks a manifest's steps and operators.
- How Meridian's DuckDB build is driven (DuckDB's own build with its extension configuration, or another route), within the parity gate.
