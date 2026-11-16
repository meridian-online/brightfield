# Brightfield's managed edition on the Mac App Store

Status: draft for Hugh's approval, 2026-11-16. Planned now; built after the signed download ships.

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
6. Runs and join proposals work under the sandbox: arc, dovetail and the SQL engine reach the granted folders.
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
- **arcform runs SQL through an engine's command-line program**, so that one manifest can target other engines. arc calls the DuckDB CLI in at least five forms: `--version`, checked against a table of known DuckDB release commits; `-noheader -list -c <sql>`; `<db> -f <file>` followed by dot-commands (`.bail off`, `.mode list`, `.headers off`, `.once '<path>'`) and a query of `duckdb_extensions()`; and `-no-init -json` and `-no-init -list -noheader` with a script on stdin, for its lineage tokenizer, which calls `sql_tokenize` from DuckDB's autocomplete extension. It sets `DUCKDB_AGENT_MODE=0` on each, and runs the program `ARC_DUCKDB_BIN` names when set. Brightfield's own engine check adds `-batch -noheader -csv -init <file> -c`. This design does not change arcform.
- **Brightfield's existing rulings hold.** Protocols are opened as folders and worked on where they are; new Protocols are made in the Protocols folder, a preference whose default is `~/Documents/Protocols`; a Protocol from a link, an archive or outside the Protocols folder opens restricted until trusted; the command line that opens things in the window is supported on builds that can offer it.
- **Build slots.** The signed download's sessions have first call on the Mac's Rust builds. The re-measure and every build task run only when a slot is free.

## Approaches

How runs reach the analyst's files was the one open choice. A run goes Brightfield → arc → the DuckDB CLI; a join proposal goes Brightfield → dovetail. arc and dovetail are Brightfield's own executable in another role, so they can resolve a bookmark handed to them. The DuckDB CLI is DuckDB's program and cannot.

- **A. A DuckDB CLI stand-in inside Brightfield (chosen).** Brightfield's executable takes a third role that is DuckDB's own shell, compiled from the pinned DuckDB's source against the DuckDB library Brightfield already links, with a preamble that resolves the grants it is handed before the shell starts. arc is pointed at it through `ARC_DUCKDB_BIN`, as it is pointed at the bundled CLI today. arc and its engine-CLI decision do not change. The risk is behaviour that differs from the real CLI; the parity gate below answers it.
- **B. arc gains an in-process DuckDB runner.** Cleaner inside arc, but it is work in a second repo, makes an exception to arc's engine-CLI decision, and puts the grants problem in arc rather than in Brightfield.
- **C. Copy the Protocol into the container, run, and copy back.** Rejected: it breaks the ruling that a Protocol is worked on where it lives, and duplicates large sources.

Hugh chose A on 2026-11-16, "as long as there are no execution parity issues". So parity is a gate, not a hope: see Execution parity.

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
- **A source or model refused during a run**, from arc, the stand-in or dovetail, comes back as a refused-access error naming the path, not as a failed step with the engine's text alone, and raises the same banner.
- **A write refused** (saving, an export, a run's outputs) does the same.

The banner and the exit below apply to both editions, since a refusal can come from macOS's privacy controls in the full edition too; the plan carries the full edition's regression checks for it.

The headless capture path keeps writing its degrade report to stderr, and gains a non-zero exit when a model read was refused, so a check reading the exit code cannot pass a half-drawn Protocol.

### 5. Children reach the grants

When Brightfield starts arc or dovetail, it passes the bookmarks for the folders the job needs (the Protocol folder, the folders of its sources, the Protocols folder) in an environment variable. Each child role resolves them and starts access before it does anything else. arc passes its environment to the programs it starts, so the stand-in receives the same bookmarks and does the same.

Each child is a helper executable inside the bundle, signed with only `app-sandbox` and `inherit`. Since the roles are Brightfield's own code, the helper is a copy of the main executable signed as a helper, under a helper bundle identifier. Whether the main executable can be started as its own child instead, which would save the copy's size, is settled by the re-measure.

arc's working databases, history and fetch cache, and DuckDB's temporary directory, live inside the container. That needs no setting: under the sandbox the home directory and temporary directory already resolve there.

### 6. The DuckDB CLI stand-in

The stand-in is DuckDB's own shell, not a reimplementation of it. The pinned DuckDB release's shell source is compiled into Brightfield and linked against the same DuckDB library the app already uses, and Brightfield's executable enters it in a third role after resolving the bookmarks it was handed. Every option, dot-command, output mode and exit status arc relies on is then DuckDB's own code, which is what makes parity reachable. `--version` must print what the pinned CLI prints, including the release commit arc checks.

The shell's DuckDB must carry the extensions the CLI has built in that arc uses, autocomplete above all, since arc's tokenizer calls `sql_tokenize`. The Store bundle ships the stand-in in place of the DuckDB CLI.

A reimplementation of the subset arc uses is the fallback only if the shell cannot be built this way, and it would have to pass the same parity gate.

### 7. Execution parity

The stand-in ships only if it is indistinguishable from the pinned DuckDB CLI on everything arc does with it:

- arc's engine and tokenizer tests, from arcform at the revision Brightfield pins, run against the stand-in through `ARC_DUCKDB_BIN` and pass as they do against the pinned CLI. This is a Brightfield CI job that checks arcform out at the pin; arcform does not change.
- Every Protocol Brightfield ships that runs offline, and the agent Protocols the repo already checks, run under each, and their outputs (tables, run records, exit statuses) compare equal, with any difference treated as a failure. A Protocol the managed edition opens restricted (for example one with a step that runs `uv`) is checked to open restricted instead.
- The forms of the CLI arc and Brightfield use, listed under Constraints, are each covered by a test.

The gate is a standing check, not a one-time one: it runs on changes to the packaging, to the stand-in, and to the arcform or DuckDB pin, and before each Store build.

If parity cannot be reached on something arc needs, the build stops there and the question comes back to Hugh with approach B as the alternative.

### 8. Steps this edition cannot run

Before a run, and on open, Brightfield asks the edition about each step: a `command:` step, a shell precondition, an operator that starts an outside program, or a DuckDB extension outside the bundled set.

Meridian's own FineType CLI is the exception among outside programs: arc's FineType operators run it, and it is Meridian's reviewed code, so the Store bundle ships it as a helper and points arc at it through `FINETYPE_BIN`. Its operators run in the managed edition. A Protocol with any of them opens restricted, through the restricted mode Brightfield already has for untrusted Protocols: its spine, steps, SQL and last results are shown, and nothing runs. The banner names the first such step and the reason, for example "Step *clean* runs `uv`, which this edition can't run", and links to the full edition on meridian.online. Trust does not lift this restriction; only the full edition does.

A Protocol made in the managed edition cannot gain such a step without being told at the moment it is added.

### 9. Extensions

The Store bundle ships a fixed set of DuckDB extensions, built for the pinned DuckDB: autocomplete (built into the shell), excel, sqlite, httpfs, ducklake, spatial, and FineType. DuckDB's extension directory points at the bundle, automatic install is off, and an `INSTALL` from the network is refused with the edition banner. FineType already loads as an unsigned extension; whether the other bundled extensions still pass DuckDB's own signature check after Apple code-signing rewrites them is a re-measure item, and if they do not, they load as unsigned from the bundle, which is the app's own signed code.

### 10. Network and Meridian's products

The managed edition carries `com.apple.security.network.client`. Links from meridian.online, archive downloads, `http_fetch` steps, httpfs and ducklake reads of managed datasets work as in the full edition. Managed Protocols and datasets are offered as they are in the full edition. Outside the United States storefront the app shows no button or link to buy Meridian Pro (guideline 3.1.1); it may encourage Meridian Public, signing in and hosted runs for a profile that has Pro. In the United States storefront a link to the web purchase is allowed.

### 11. Opening from Terminal

The managed edition does not read paths from its own command line. `open -a Brightfield <path>` goes through the same macOS route as a Finder double-click, and is expected to grant the path; the re-measure confirms it. The documentation for the managed edition says so, and gives the one-line shell function an analyst can add themselves. The capture harness's command line is unchanged in the full edition.

### 12. Packaging

A Store packaging path beside the existing one:

- Signing for the Store runs in CI, beside the release and signing-rehearsal workflows, with the Store certificates and profile as secrets.
- An entitlements file for the app (`app-sandbox`, `files.user-selected.read-write`, `files.bookmarks.app-scope`, `files.downloads.read-write`, `network.client`) and one for the helper (`app-sandbox`, `inherit`).
- The bundle holds the main executable, the helper, FineType's CLI as a second helper, the bundled extensions under `Contents/PlugIns` or `Contents/Resources`, FineType's model, and no DuckDB CLI.
- Signing inside out with the Mac App Distribution certificate and the Mac App Store provisioning profile embedded, then a `.pkg` signed with the Mac Installer Distribution certificate, uploaded to App Store Connect.
- `ITSAppUsesNonExemptEncryption` false: Brightfield uses only the system's HTTPS.
- The same bundle identifier as the full edition, `online.meridian.brightfield`. Apple says the two builds do not share privacy grants, which is expected; whether a shared identifier also makes the two share a container or a Launch Services registration in a way that confuses an analyst with both installed is checked in the re-measure, and a separate identifier is the fallback.

## The re-measure

The v0.1.2 measurement is three months old, and opening and saving have been rebuilt since. Before any build task, re-measure today's `main` under App Sandbox, ad-hoc signed as the v0.1.2 run was (no Apple enrolment needed; the kernel applies the sandbox from the signature). It runs when a build slot is free, and its results revise this design before the build starts. The detector is the window's own report plus the headless degrade line on stderr, as in v0.1.2.

Baseline, today's build sandboxed:

1. Open a Protocol folder from the open panel.
2. Open its `arcform.yaml` alone from the panel.
3. Open the folder and the file through Finder's Open With.
4. Save an edit.
5. Relaunch and reopen from Recents.
6. Open with `models/` at mode `000`.
7. Run the Protocol, and propose joins on a folder of tables.

Premises the design rests on, each a small spike:

8. A bookmark passed in the environment, resolved by a child signed with `inherit`, grants the child the folder; and the same for a grandchild started by that child.
9. The main executable started as its own child under the sandbox: allowed, and with which rights.
10. `open -a` from Terminal grants the path.
11. `/bin/sh` and the system's own tools run under the sandbox (for knowing exactly what the restriction removes).
12. A bundled DuckDB extension, signed by Apple's `codesign`, loads with DuckDB's signature check on.
13. Making Brightfield the default app for CSV, Parquet and DuckDB still works sandboxed.
14. A shared bundle identifier with the full edition installed beside it.

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

DuckDB's extension downloads conflict with 2.4.5(iv) and 2.5.2. The managed edition ships its extensions inside the app and does not download any (design section 9). FineType's model already ships in the bundle. Data downloads, such as a Protocol's sources, a managed dataset or an archive from meridian.online, are data, not code, and are unaffected.

### Signing the nested executables

The helpers (Brightfield's executable in its arc, dovetail and stand-in roles, and FineType's CLI) are signed with `app-sandbox` and `inherit` only. The bundled extensions, including FineType, are signed with the distribution identity. Signing runs inside out: extensions and helper first, then the app. The DuckDB CLI is not in the Store bundle, so its upstream signature no longer matters there. Hardened runtime is not required for the Store; leaving library validation as Apple's default is checked against the extensions in the re-measure.

## Hugh's rulings in this brainstorm, 2026-11-16

- No channel may cost the analyst capability "is an important principle", and Hugh is "prepared to be pragmatic".
- Meridian Pro: "Pro will be a web product, tied to a profile. I don't think we need to commit to in-app purchases."
- Editions: full through Homebrew, managed through the App Store for environments "where arbitrary code can't run", and a viewer edition for the App Store and iPad. Names are open.
- IT policies beyond the sandbox are deferred to research or a later brainstorm.
- Every edition makes Meridian's managed Protocols and datasets easy and encouraged, while local development and runs stay the default where they make sense: "We want to grow our business but not while displacing customer delight."
- Approach A, "as long as there are no execution parity issues".
- A Protocol the managed edition cannot run opens restricted and names the step and the full edition ("This looks like the right approach").

## Calls made on Hugh's behalf

- Only the Protocols folder is asked at first launch; every other grant is asked when it is needed.
- The full edition's command steps and shell preconditions are both refused in the managed edition, since a precondition is arbitrary shell too.
- Bundled extensions: excel, sqlite, httpfs, ducklake, spatial, FineType. Spatial is large; it is in because a shipped Protocol loads it.
- The managed edition does not import a full edition's preferences, Recents or trust.
- The same bundle identifier as the full edition, subject to the re-measure.
- The headless path exits non-zero on a refused model read.
- Trust does not lift the edition restriction.

- The stand-in is DuckDB's own shell compiled into Brightfield, not a reimplementation.
- FineType's CLI ships as a helper, so arc's FineType operators run in the managed edition.
- The refused-access banner and the headless exit apply to both editions.
- The plan runs to the milestone's end: an App Store submission and the run that proves it on a Mac that has never had Brightfield. Signing runs in CI.

## Left to the builder

- The grant store's file format, and how stale bookmarks are refreshed.
- The environment variable's name and encoding for passing bookmarks.
- The banner wording, within the rule that it names the path or step and the action.
- Where the bundled extensions sit inside the bundle.
- How the restricted check walks a manifest's steps and operators.
- The stand-in's code layout, within the parity gate.
