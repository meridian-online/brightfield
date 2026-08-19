//! Gate: no two panes the window can place declare the same
//! [`StatusEntry`](brightfield_workbench::StatusEntry) id.
//!
//! The id is a stable name and nothing dedups by it. `chrome::status_rail`
//! draws each entry it is handed, in order, so two entries sharing an id draw
//! twice — `chrome_rules.rs::two_entries_sharing_an_id_both_draw` is that
//! behaviour pinned. Since the window collects the status lines of every
//! placed pane rather than the focused one's, two panes of one view that
//! declare the same id put the same line on the rail twice.
//!
//! There is a second, quieter failure: `Activity::of_entry` recognises a
//! pane's activity report by matching its id against the ids in
//! `Activity::ALL`, and the window filters those out of the rail because the
//! merged indicator says them. A new line colliding with one of those ids
//! would be filtered out rather than drawn — the test
//! `status_rail.rs::activity_reaches_the_rail_as_the_one_indicator` holds
//! that filtering.
//!
//! # The property is asserted by running the panes, not by reading them
//!
//! This file first tried to answer the question from source text: find every
//! string literal handed to a `StatusEntry` id and report a repeat. Review
//! found four ways a real duplicate reaches the rail while such a scan says
//! the tree is clean, and they are one shape — **a source-text scan cannot
//! see through indirection.** A second caller reached through
//! `use crate::rail::run_line as owns_it;` contains no registered call
//! fragment. One pane's `describe` calling another's and folding the returned
//! `Subject`'s entries into its own contains no `StatusEntry` text at all.
//! Alias, delegation, re-export, trait dispatch, macro, closure: same
//! problem, different clothes, and patching two of them buys finding the next
//! two.
//!
//! So the gate constructs the panes and asks them. [`ItemSpec::make`] is a
//! `fn() -> Box<dyn Item<D>>` per registered pane, so a test can build every
//! pane a view can place, call the real `Item::subject`, and read the ids
//! that come back. Indirection is invisible to that, because it runs the code
//! instead of reading it. It is the same move this file already made for
//! `Activity::ALL`, whose ids are match arms rather than construction-site
//! literals: link the crate and ask.
//!
//! [`ItemSpec::make`]: brightfield_workbench::registry::ItemSpec::make
//!
//! # What runtime does not reach, and what covers it
//!
//! **A branch no fixture reaches.** `describe` is a function of the document,
//! and several rail lines are conditional. [`chart_documents`] drives the
//! matrix that reaches what it can; [`RAIL_IDS`] marks each id either
//! [`Reach::Observed`] or [`Reach::Declared`] with the reason no fixture
//! places it, and the gate asserts the observed set is *exactly* the set
//! marked observed — so a fixture that stops working reddens rather than
//! quietly narrowing the check.
//!
//! **Pane-local state.** `(spec.make)()` builds a fresh pane, so an id gated
//! on state the pane accumulates — the spec editor's `saved` and `warning`
//! lines hang off a file it has been given — cannot be reached this way.
//!
//! **A surface that is not a registered pane.** The window composes two rail
//! lines itself. [`a_booted_window_draws_no_id_twice`] covers those by booting
//! the real window and reading `MeridianApp::rail`, which is what actually
//! drew.
//!
//! **An entry that never reaches a `Subject`.** The dev gallery builds two
//! specimens and hands them straight to `chrome::status_rail` on its own
//! surface, so no pane declares them and nothing can collide with them there.
//!
//! For the first two of those, the residual at the foot of this file reads the
//! id literals it can see in `crates/*/src` and requires each to be declared
//! in [`RAIL_IDS`]. That is a rot-check on the table, **not** the uniqueness
//! property: it is the half that catches a *new* id nobody registered, on a
//! branch nothing runs. It is blind to exactly the indirection described
//! above — which is why it no longer carries the property, and why the two
//! halves divide the way they do. An alias or a delegation creates no new
//! literal, so text cannot see it and runtime can; a literal on an
//! unreachable branch runs nowhere, so runtime cannot see it and text can.
//!
//! What neither reaches: a **second** declaration of an **already-declared**
//! id, on a branch no fixture runs. Named here rather than implied.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::{chart_registry_with, ChartDoc};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::{
    load_protocol_offline, protocol_registry, ProtocolDoc, ProtocolModel,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::registry::ItemRegistry;
use brightfield_workbench::subject::RunState;
use brightfield_workbench::{Activity, Item, ItemId, HONESTY_LINE_MS};

const DASHBOARD: &str = "../../examples/dashboard.yaml";
const EDGAR: &str = "../../examples/protocol/edgar_gleif/arcform.yaml";

// ---------------------------------------------------------------------------
// The declared id space
// ---------------------------------------------------------------------------

/// Whether the fixtures in this file place an id, or why they do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// A fixture places it, and the gate requires that it still does — an
    /// entry that stops being observed reddens, so the matrix cannot rot into
    /// a check that asks less than it says.
    Observed,
    /// No fixture places it, and this is why.
    Declared(&'static str),
}

/// The status rail's id space: every id a surface may put on the rail, the
/// one surface allowed to put it there, and whether this file reaches it.
///
/// Before this table nothing declared the id space at all — the ids were
/// string literals at their call sites and the uniqueness requirement lived
/// in a doc comment. The owner column is what turns "this id appeared twice"
/// into "this id appeared somewhere it does not belong", which is the report
/// a reader can act on.
///
/// A surface that is not a pane is named in angle brackets, so it cannot be
/// confused with an `ItemId`.
const RAIL_IDS: &[(&str, &str, Reach)] = &[
    ("run-state", "chart-canvas", Reach::Observed),
    (
        "chart-navigation",
        "chart-canvas",
        Reach::Declared("needs a navigation gesture refused on a live session"),
    ),
    (
        "chart-navigation-scope",
        "chart-canvas",
        Reach::Declared("needs a plot held at an extent whose mark declined to rescope"),
    ),
    (
        "chart-predicate",
        "chart-canvas",
        Reach::Declared("needs a committed selection on a live session"),
    ),
    ("activity-engine-query", "chart-canvas", Reach::Observed),
    ("activity-protocol-run", "chart-canvas", Reach::Observed),
    ("activity-file-watch", "chart-canvas", Reach::Observed),
    ("watch-spec", "chart-canvas", Reach::Observed),
    ("watch-data", "chart-canvas", Reach::Observed),
    (
        "editor-saved",
        "spec-editor",
        Reach::Declared("pane-local state: a pane built by `make` has been given no file"),
    ),
    (
        "editor-warning",
        "spec-editor",
        Reach::Declared("pane-local state, as above"),
    ),
    ("chart-idle", "<window>", Reach::Observed),
    ("activity", "<window>", Reach::Observed),
    (
        "gallery-status-rail-predicate",
        "<dev gallery>",
        Reach::Declared("a specimen handed to `chrome::status_rail`, never to a Subject"),
    ),
    (
        "gallery-status-rail-idle",
        "<dev gallery>",
        Reach::Declared("a specimen, as above"),
    ),
];

/// The owner declared for `id`, if the table has one.
fn declared_owner(id: &str) -> Option<&'static str> {
    RAIL_IDS
        .iter()
        .find(|(known, _, _)| *known == id)
        .map(|(_, owner, _)| *owner)
}

// ---------------------------------------------------------------------------
// Running the panes
// ---------------------------------------------------------------------------

/// One rail line a pane placed: which pane, which id.
type Placed = (ItemId, &'static str);

/// Every rail line every registered pane of `registry` places over `doc`.
///
/// Registered rather than currently placed, which is the conservative
/// direction: `ItemRegistry::default_tree` gives each spec a tile, and the
/// toggles only ever *remove* panes from that, so checking the whole registry
/// checks a superset of any arrangement the user can reach.
fn placed<D: ?Sized>(registry: &ItemRegistry<D>, doc: &D) -> Vec<Placed> {
    let mut out = Vec::new();
    for spec in registry.specs() {
        let pane: Box<dyn Item<D>> = (spec.make)();
        for entry in pane.subject(doc).status {
            out.push((spec.id, entry.id));
        }
    }
    out
}

/// Everything wrong with what one view placed over one document.
fn complaints(view: &str, doc: &str, lines: &[Placed]) -> Vec<String> {
    let mut out = Vec::new();
    let mut first: BTreeMap<&str, ItemId> = BTreeMap::new();
    for (pane, id) in lines {
        if let Some(prev) = first.insert(id, *pane) {
            out.push(format!(
                "{view} over {doc}: `{id}` is declared by {prev} and by {pane} — the rail \
                 draws each entry it is handed, so that line appears twice"
            ));
        }
        match declared_owner(id) {
            None => out.push(format!(
                "{view} over {doc}: {pane} declares `{id}`, which RAIL_IDS does not know — \
                 add it with the surface that owns it"
            )),
            Some(owner) if owner != pane.as_str() => out.push(format!(
                "{view} over {doc}: {pane} declares `{id}`, which RAIL_IDS gives to {owner}"
            )),
            Some(_) => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The documents
// ---------------------------------------------------------------------------

/// A scratch directory unique to this test binary.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("bf-rail-ids-{}", std::process::id()))
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// The real dashboard, annotated with the run output `state`.
fn annotated(state: Option<RunState>) -> ChartDoc {
    let mut composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    if let Some(state) = state {
        composed = composed.with_run_state(state);
    }
    let mut doc = ChartDoc::headless(composed);
    doc.spec_path = Some(DASHBOARD.into());
    doc
}

/// The dashboard with a spec file and a data file that have both moved under
/// the watcher — the two file notices, without a window to boot.
fn watched() -> ChartDoc {
    let dir = scratch("watched");
    let spec = dir.join("spec.yaml");
    let data = dir.join("rows.csv");
    fs::write(&spec, include_str!("../../../examples/dashboard.yaml")).expect("spec");
    fs::write(&data, "a,b\n1,2\n").expect("data");

    let mut doc = annotated(Some(RunState::Fresh));
    doc.watch.watch(Some(spec.clone()), vec![data.clone()]);
    for path in [&spec, &data] {
        let f = fs::File::options().write(true).open(path).expect("reopen");
        f.set_modified(SystemTime::now() - Duration::from_secs(120))
            .expect("move the mtime somewhere the baseline is not");
    }
    assert!(
        doc.watch.poll_now(),
        "the watcher saw no change, so this document reaches neither file notice"
    );
    doc
}

/// The Charts documents the gate drives every pane over, each with the name
/// that appears in a complaint.
fn chart_documents() -> Vec<(String, ChartDoc)> {
    let mut out: Vec<(String, ChartDoc)> = vec![
        ("an empty document".to_string(), ChartDoc::empty()),
        (
            "the dashboard with no run recorded".to_string(),
            annotated(None),
        ),
    ];
    for state in RunState::ALL {
        out.push((
            format!("the dashboard recorded {state:?}"),
            annotated(Some(state)),
        ));
    }
    for kind in Activity::ALL {
        let mut doc = annotated(Some(RunState::Fresh));
        doc.activity.begin(kind);
        out.push((format!("the dashboard with {kind:?} in flight"), doc));
    }
    out.push((
        "the dashboard with both files moved on disk".to_string(),
        watched(),
    ));
    out
}

/// The Protocol documents. The panel declares no status entry today; driving
/// it anyway is what makes the first one somebody adds arrive through this
/// gate rather than past it.
fn protocol_documents() -> Vec<(String, ProtocolDoc)> {
    let inputs = load_protocol_offline(EDGAR).expect("load the offline protocol fixture");
    vec![
        ("an empty document".to_string(), ProtocolDoc::empty()),
        (
            "the offline protocol fixture".to_string(),
            ProtocolDoc::headless(ProtocolModel::new(inputs, Flow::Vertical)),
        ),
    ]
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

#[test]
fn no_two_panes_declare_the_same_rail_id() {
    let charts = chart_documents();
    let protocol = protocol_documents();
    // The activity log owes an entry only past the honesty line, so the docs
    // that begin work are built first and read after it has passed.
    std::thread::sleep(Duration::from_millis(
        u64::try_from(HONESTY_LINE_MS).expect("small") + 40,
    ));

    let mut found = Vec::new();
    let mut observed: BTreeSet<&'static str> = BTreeSet::new();

    for gallery in [false, true] {
        let registry = chart_registry_with(gallery);
        let view = if gallery {
            "the Charts view with the dev gallery"
        } else {
            "the Charts view"
        };
        for (name, doc) in &charts {
            let lines = placed(&registry, doc);
            observed.extend(lines.iter().map(|(_, id)| *id));
            found.extend(complaints(view, name, &lines));
        }
    }
    let registry = protocol_registry();
    for (name, doc) in &protocol {
        let lines = placed(&registry, doc);
        observed.extend(lines.iter().map(|(_, id)| *id));
        found.extend(complaints("the Protocol view", name, &lines));
    }

    // The window's own two lines, from the window that actually drew them.
    observed.extend(window_drew());

    assert!(
        found.is_empty(),
        "status-rail id violations ({}):\n{}",
        found.len(),
        found.join("\n")
    );

    // And the matrix still reaches what the table says it reaches. Without
    // this the check narrows silently: a fixture that stopped placing its
    // entry would leave the assertion above green over less and less.
    let want: BTreeSet<&str> = RAIL_IDS
        .iter()
        .filter(|(_, _, reach)| *reach == Reach::Observed)
        .map(|(id, _, _)| *id)
        .collect();
    let got: BTreeSet<&str> = observed.iter().copied().collect();
    let missing: Vec<&&str> = want.difference(&got).collect();
    let extra: Vec<&&str> = got.difference(&want).collect();
    assert!(
        missing.is_empty(),
        "RAIL_IDS marks these Observed but no fixture placed them, so the gate \
         is checking less than it claims: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "these ids were placed but RAIL_IDS calls them unreachable — mark them \
         Observed: {extra:?}"
    );
}

/// The rail a booted window actually drew, which is where the two lines the
/// window composes itself — the idle line and the merged activity indicator —
/// can collide with a pane's.
fn window_drew() -> Vec<&'static str> {
    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let mut boot = Boot::charts(composed);
    boot.spec_path = Some(DASHBOARD.into());
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0));
    let mut frame = |app: &mut MeridianApp| {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    };
    frame(&mut app);
    frame(&mut app);
    let mut drew = app.rail().drawn.clone();

    // Again with work in flight, which is what summons the indicator.
    app.chart_doc_mut().activity.begin(Activity::EngineQuery);
    std::thread::sleep(Duration::from_millis(
        u64::try_from(HONESTY_LINE_MS).expect("small") + 40,
    ));
    frame(&mut app);
    drew.extend(app.rail().drawn.iter().copied());
    drew
}

#[test]
fn a_booted_window_draws_no_id_twice() {
    let drew = window_drew();
    let mut seen = BTreeSet::new();
    let repeated: Vec<&&str> = drew.iter().filter(|id| !seen.insert(**id)).collect();
    assert!(
        repeated.is_empty(),
        "the window drew {repeated:?} more than once; it drew {drew:?}"
    );
    for id in &drew {
        assert!(
            declared_owner(id).is_some(),
            "the window drew `{id}`, which RAIL_IDS does not know; it drew {drew:?}"
        );
    }
}

#[test]
fn the_declared_id_space_names_each_id_once() {
    let mut seen = BTreeSet::new();
    for (id, owner, reach) in RAIL_IDS {
        assert!(
            seen.insert(*id),
            "RAIL_IDS names `{id}` twice, so it cannot say who owns it"
        );
        assert!(!owner.trim().is_empty(), "`{id}` has no owner");
        if let Reach::Declared(why) = reach {
            assert!(
                !why.trim().is_empty(),
                "`{id}` is out of the fixtures' reach and does not say why"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The residual: id literals the runtime matrix never runs
// ---------------------------------------------------------------------------
//
// Everything below is a rot-check on RAIL_IDS, not the uniqueness property.
// It reads the id literals it can see in `crates/*/src` and requires each to
// be declared. Its job is the one thing running the panes cannot do: notice a
// NEW id added on a branch no fixture reaches.
//
// It is blind to indirection, by construction and on purpose. An alias, a
// delegation, a re-export or a macro creates no new literal, so there is
// nothing here for it to see — and nothing it needs to see, because those
// reuse an id that already exists and the runtime half catches the reuse. The
// division is deliberate: this half is for literals that never run, the
// runtime half is for code that runs however it was reached.

/// A site whose id the scan cannot follow, and why that is safe:
/// `(path suffix, expression as written, why)`.
type Allow = (&'static str, &'static str, &'static str);

/// How many unreadable sites may be excused.
const ALLOW_CAP: usize = 5;

/// The sites whose id expression the scan cannot follow.
const UNREADABLE_ALLOW: &[Allow] = &[
    (
        "crates/brightfield-workbench/src/subject.rs",
        "id",
        "RunState::status_entry forwards the id its caller passed, so the \
         literal is at the call and the scan reads it there.",
    ),
    (
        "crates/brightfield-workbench/src/activity.rs",
        "self.id()",
        "Activity::id is a match over Activity::ALL. Those ids reach RAIL_IDS \
         through the runtime half, which links the crate rather than reading \
         the arms.",
    ),
];

/// How far past a `StatusEntry {` the `id` field may sit before the scan gives
/// up and reports.
const FIELD_SCAN: usize = 12;

/// The fewest id literals the workspace may hold before the scan is assumed to
/// have stopped reading rather than the rail to have gone quiet.
const SITE_FLOOR: usize = 10;

/// One place production code hands a `StatusEntry` an id.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Site {
    /// Workspace-relative path of the file the site is in.
    rel: String,
    /// Workspace-relative path and 1-based line.
    at: String,
    /// The id expression exactly as written.
    expr: String,
}

/// If `lines[i]` opens a `#[cfg(test)]` **module** in column 0, the index just
/// past that module's closing brace.
///
/// `None` when the attribute sits on a bare item — a `const`, a `use`, a `fn`
/// — because such an item opens no module and closes no brace of its own. An
/// earlier draft skipped to the next column-0 `}` regardless, which on a bare
/// item ran past every declaration up to whatever brace came next and took
/// them with it. `brightfield-bench/src/main.rs` carries four consecutive bare
/// `#[cfg(test)] const` items, so the shape is in this tree;
/// `a_bare_cfg_test_item_does_not_hide_what_follows_it` holds it.
fn test_module_end(lines: &[&str], i: usize) -> Option<usize> {
    if lines[i] != "#[cfg(test)]" {
        return None;
    }
    let mut j = i + 1;
    while j < lines.len() {
        let trimmed = lines[j].trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#[") {
            j += 1;
            continue;
        }
        break;
    }
    let decl = lines.get(j)?.trim_start();
    // `mod x;` names a file and opens no body, so the next brace is not its.
    let opens_a_body = decl.ends_with('{');
    if !(opens_a_body && (decl.starts_with("mod ") || decl.starts_with("pub mod "))) {
        return None;
    }
    let mut k = j;
    while k < lines.len() && lines[k] != "}" {
        k += 1;
    }
    Some(k + 1)
}

/// The argument of a `…status_entry(<expr>)` call on `line`.
///
/// The needle is anchored at the front: a name that merely *ends* in
/// `status_entry` is a different function. `idle_status_entry` and
/// `draw_status_entry` are both in this workspace, and an unanchored needle
/// read their first argument as an id.
fn call_argument(line: &str) -> Option<String> {
    if line.contains("fn status_entry(") {
        return None;
    }
    let idx = line
        .match_indices("status_entry(")
        .find(|(at, _)| {
            line[..*at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
        })
        .map(|(at, _)| at)?;
    let rest = &line[idx + "status_entry(".len()..];
    let close = rest.find(')')?;
    let arg = rest[..close].trim();
    (!arg.is_empty()).then(|| arg.to_string())
}

/// The `id` field belonging to the `StatusEntry {` that opens at `open`.
fn field_site(rel: &str, lines: &[&str], open: usize) -> Site {
    let end = (open + 1 + FIELD_SCAN).min(lines.len());
    for (n, line) in lines.iter().enumerate().take(end).skip(open + 1) {
        let trimmed = line.trim();
        let expr = if trimmed == "id," {
            // Field shorthand: the id is whatever local named `id` holds.
            "id".to_string()
        } else if let Some(rest) = trimmed.strip_prefix("id:") {
            rest.trim().trim_end_matches(',').trim().to_string()
        } else {
            continue;
        };
        return Site {
            rel: rel.to_string(),
            at: format!("{rel}:{}", n + 1),
            expr,
        };
    }
    Site {
        rel: rel.to_string(),
        at: format!("{rel}:{}", open + 1),
        expr: format!("<no id field within {FIELD_SCAN} lines of the construction>"),
    }
}

/// Every id-supplying site in one file's text.
fn sites(rel: &str, text: &str) -> Vec<Site> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(past) = test_module_end(&lines, i) {
            i = past;
            continue;
        }
        let line = lines[i];
        if line.trim_start().starts_with("//") {
            i += 1;
            continue;
        }
        // `-> StatusEntry {` opens a function body, not a construction, and
        // the construction it returns is on a line of its own.
        if line.contains("StatusEntry {")
            && !line.contains("-> StatusEntry {")
            && !line.contains("struct StatusEntry")
        {
            out.push(field_site(rel, &lines, i));
        }
        if let Some(expr) = call_argument(line) {
            out.push(Site {
                rel: rel.to_string(),
                at: format!("{rel}:{}", i + 1),
                expr,
            });
        }
        i += 1;
    }
    out
}

/// The name and value of a `const NAME: &str = "…";` on `line`.
fn constant_decl(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let trimmed = trimmed
        .strip_prefix("pub(crate) ")
        .or_else(|| trimmed.strip_prefix("pub "))
        .unwrap_or(trimmed);
    let rest = trimmed.strip_prefix("const ")?;
    let (name, rest) = rest.split_once(':')?;
    let (ty, value) = rest.split_once('=')?;
    if !matches!(ty.trim(), "&str" | "&'static str") {
        return None;
    }
    let inner = value.trim().strip_prefix('"')?;
    let end = inner.find('"')?;
    if !inner[end + 1..].trim().starts_with(';') {
        return None;
    }
    Some((name.trim().to_string(), inner[..end].to_string()))
}

/// Every readable string constant, as `name -> (file -> value)`.
fn constants(files: &[(String, String)]) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (rel, text) in files {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if let Some(past) = test_module_end(&lines, i) {
                i = past;
                continue;
            }
            if let Some((name, value)) = constant_decl(lines[i]) {
                out.entry(name).or_default().insert(rel.clone(), value);
            }
            i += 1;
        }
    }
    out
}

/// What an id expression turned out to be.
enum Read {
    /// The id itself.
    Id(String),
    /// Why the scan could not follow the expression.
    Unreadable(String),
}

/// Follow one site's expression to the id it supplies.
fn read_id(site: &Site, consts: &BTreeMap<String, BTreeMap<String, String>>) -> Read {
    if let Some(inner) = site.expr.strip_prefix('"') {
        return match inner.strip_suffix('"') {
            Some(body) if !body.contains('"') => Read::Id(body.to_string()),
            _ => Read::Unreadable(format!("not a plain string literal: `{}`", site.expr)),
        };
    }
    let name = site.expr.rsplit("::").next().unwrap_or(&site.expr);
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Read::Unreadable(format!(
            "neither a string literal nor a constant name: `{}`",
            site.expr
        ));
    }
    let Some(declared) = consts.get(name) else {
        return Read::Unreadable(format!(
            "`{}` is not a string constant this scan read",
            site.expr
        ));
    };
    if let Some(value) = declared.get(&site.rel) {
        return Read::Id(value.clone());
    }
    let distinct: BTreeSet<&String> = declared.values().collect();
    match distinct.len() {
        1 => Read::Id((*distinct.iter().next().expect("one value")).clone()),
        _ => Read::Unreadable(format!(
            "`{}` names {} different constants, so this scan cannot say which \
             one the site means",
            site.expr,
            distinct.len()
        )),
    }
}

/// Every id literal in `files` that RAIL_IDS does not declare, plus every site
/// whose expression the scan could not follow and no entry in `allow` excuses.
fn undeclared(files: &[(String, String)], allow: &[Allow]) -> Vec<String> {
    let consts = constants(files);
    let mut out = Vec::new();
    let mut excuses_used: BTreeSet<usize> = BTreeSet::new();
    for (rel, text) in files {
        for site in sites(rel, text) {
            match read_id(&site, &consts) {
                Read::Id(id) => {
                    if declared_owner(&id).is_none() {
                        out.push(format!(
                            "{}: `{id}` is handed to a StatusEntry and RAIL_IDS does not \
                             know it — declare it with the surface that owns it",
                            site.at
                        ));
                    }
                }
                Read::Unreadable(why) => {
                    if let Some(index) = allow
                        .iter()
                        .position(|(s, e, _)| site.rel.ends_with(s) && site.expr == *e)
                    {
                        excuses_used.insert(index);
                        continue;
                    }
                    out.push(format!("{}: unreadable status-entry id — {why}", site.at));
                }
            }
        }
    }
    for (index, (suffix, expr, _)) in allow.iter().enumerate() {
        if !excuses_used.contains(&index) {
            out.push(format!(
                "UNREADABLE_ALLOW entry ({suffix}, {expr}) excused nothing — the site \
                 it was written for is gone, so the entry is too"
            ));
        }
    }
    out.sort();
    out
}

fn rs_files(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.expect("dir entry").path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .expect("a read entry has a name")
            .to_string_lossy()
            .to_string();
        if path.is_dir() {
            rs_files(&path, &format!("{prefix}/{name}"), out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).expect("source file is UTF-8");
            out.push((format!("{prefix}/{name}"), text));
        }
    }
}

/// Every production source file in the workspace, as `(relative path, text)`.
fn production_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("this crate sits at <workspace>/crates/<name>")
        .to_path_buf();
    let mut out = Vec::new();
    let mut crates: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .expect("crates/ is readable")
        .map(|e| e.expect("dir entry").path())
        .collect();
    crates.sort();
    for dir in crates {
        let src = dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let name = dir
            .file_name()
            .expect("a crate directory has a name")
            .to_string_lossy()
            .to_string();
        rs_files(&src, &format!("crates/{name}/src"), &mut out);
    }
    out
}

#[test]
fn every_id_literal_in_the_workspace_is_declared() {
    let files = production_sources();
    assert!(
        files.len() > 50,
        "the scan found {} source files, which is too few to have walked the workspace",
        files.len()
    );
    let all: Vec<Site> = files
        .iter()
        .flat_map(|(rel, text)| sites(rel, text))
        .collect();
    assert!(
        all.len() >= SITE_FLOOR,
        "the scan found {} id sites in the workspace, under the floor of \
         {SITE_FLOOR} — its needles have stopped matching",
        all.len()
    );

    let found = undeclared(&files, UNREADABLE_ALLOW);
    assert!(
        found.is_empty(),
        "undeclared status-rail ids ({}):\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn the_allowlist_stays_small_and_justified() {
    assert!(
        UNREADABLE_ALLOW.len() <= ALLOW_CAP,
        "UNREADABLE_ALLOW holds {} entries, past the cap of {ALLOW_CAP}",
        UNREADABLE_ALLOW.len()
    );
    for (suffix, expr, why) in UNREADABLE_ALLOW {
        assert!(
            !why.trim().is_empty(),
            "UNREADABLE_ALLOW entry ({suffix}, {expr}) carries no reason"
        );
    }
}

// ---------------------------------------------------------------------------
// The residual proves it can still fail, on synthetic source
// ---------------------------------------------------------------------------

fn synthetic(files: &[(&str, &str)]) -> Vec<String> {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(rel, text)| ((*rel).to_string(), (*text).to_string()))
        .collect();
    undeclared(&owned, &[])
}

#[test]
fn a_bare_cfg_test_item_does_not_hide_what_follows_it() {
    let bare = r#"
#[cfg(test)]
const FIXTURE: &str = "a fixture compiled in for tests";

impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            id: "not-a-declared-id",
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", bare)]);
    assert_eq!(
        found.len(),
        1,
        "a bare `#[cfg(test)]` item hid the declaration after it; got {found:?}"
    );
    assert!(found[0].contains("`not-a-declared-id`"), "{}", found[0]);
}

#[test]
fn a_fixture_inside_a_test_module_is_not_a_declaration() {
    let with_tests = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn a_fixture() {
        let _ = StatusEntry {
            id: "not-a-declared-id",
            side: StatusSide::Trailing,
        };
    }
}
"#;
    assert!(
        synthetic(&[("crates/a/src/chart.rs", with_tests)]).is_empty(),
        "a fixture in a test module was read as a declaration"
    );
}

#[test]
fn a_declared_id_is_not_a_finding() {
    let declared = r#"
impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(state.status_entry("run-state"))
    }
}
"#;
    assert!(synthetic(&[("crates/a/src/chart.rs", declared)]).is_empty());
}

#[test]
fn an_id_reached_through_a_constant_is_followed() {
    let by_const = r#"
const MADE_UP: &str = "not-a-declared-id";

impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            id: MADE_UP,
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", by_const)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(found[0].contains("`not-a-declared-id`"), "{}", found[0]);
}

#[test]
fn a_longer_name_ending_in_status_entry_is_a_different_call() {
    // Both of these are real lines in this workspace, and the first version of
    // this scan read the argument of each as a status-entry id.
    let neighbours = r#"
fn idle_status_entry(composed: &Composed) -> Option<StatusEntry> {
    None
}

fn rail(&mut self) {
    if let Some(idle) = idle_status_entry(&self.charts.doc.composed) {
        entries.push(idle);
    }
    draw_status_entry(ui, entry, mode, &mut out);
}
"#;
    assert_eq!(
        synthetic(&[("crates/a/src/window.rs", neighbours)]),
        Vec::<String>::new(),
        "a function whose name merely ends in `status_entry` was read as a declaration"
    );
}

#[test]
fn an_id_the_scan_cannot_follow_is_reported() {
    let computed = r#"
impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            id: leak(&format!("pane-{n}")),
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", computed)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].contains("unreadable status-entry id"),
        "an id built at run time went unreported: {}",
        found[0]
    );
}

#[test]
fn a_construction_with_no_id_field_is_reported() {
    let headless = r#"
impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", headless)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(found[0].contains("no id field"), "{}", found[0]);
}

#[test]
fn an_allowlist_entry_that_excuses_nothing_is_reported() {
    let owned = vec![("crates/a/src/chart.rs".to_string(), String::new())];
    let stale: &[Allow] = &[("crates/a/src/gone.rs", "self.id()", "a site since deleted")];
    let found = undeclared(&owned, stale);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(found[0].contains("excused nothing"), "{}", found[0]);
}
