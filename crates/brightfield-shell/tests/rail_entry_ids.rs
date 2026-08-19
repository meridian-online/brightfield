//! Gate: two places in the workspace's production code may not hand a
//! [`StatusEntry`](brightfield_workbench::StatusEntry) the same id.
//!
//! The id is a stable name and nothing dedups by it. `chrome::status_rail`
//! draws each entry it is handed, in order, so two entries sharing an id draw
//! twice — `chrome_rules.rs::two_entries_sharing_an_id_both_draw` is that
//! behaviour pinned. Since the window collects the status lines of every
//! placed pane rather than the focused one's, two panes in one view that
//! declare the same id put the same line on the rail twice.
//!
//! There is a second, quieter failure: `Activity::of_entry` recognises a
//! pane's activity report by matching its id against
//! [`Activity::ALL`](brightfield_workbench::Activity::ALL), and the window
//! filters those out of the rail because the merged indicator says them. A
//! new line that collided with one of those three ids would be filtered out
//! and never drawn at all.
//!
//! # Why this reads source rather than a rendered frame
//!
//! A test that boots a window and inspects `MeridianApp::rail().drawn` sees
//! the entries a document's current state happens to raise. Most rail lines
//! are conditional — a navigation refusal needs a refused gesture, the
//! watcher's notice needs a file to move — so a frame-driven test covers the
//! branches its fixture reaches and is silent about the rest. The collision
//! this file was written for lived in exactly such a gap: `run-state` was
//! declared by two panes and unreachable in production, because the run
//! ingestion path that sets `Composed::run_state` has no caller outside
//! tests. Reading the declarations catches a collision whether or not
//! anything can currently reach it.
//!
//! `chart_contract.rs::the_charts_view_declares_no_duplicate_status_id` is
//! the other half: it drives the real panes over a document that does carry a
//! run state and compares what they declare. This file is the one that can
//! see a pane it has never been told about.
//!
//! # What the scan reads
//!
//! Every `.rs` file under `crates/*/src/`, minus the `#[cfg(test)]` modules
//! that start in column 0 (which is where this workspace puts them: measured
//! on this tree, 129 of the 138 `#[cfg(test)]` attributes under `crates/*/src`
//! are unindented, and none of the 9 indented ones is in a file that
//! constructs a `StatusEntry`). Two shapes supply an id:
//!
//! * a `StatusEntry { … }` construction — the `id` field that follows it;
//! * a `…status_entry(<expr>)` call — its argument. `Activity::status_entry`
//!   takes no argument and so supplies nothing here; its id comes from
//!   `Activity::id`, which this test reads from the linked crate instead.
//!
//! An id expression is read when it is a string literal, or a name bound by a
//! `const … : &str = "…"` this scan also read. **Anything else is a
//! violation**, not a shrug: an id the scan cannot follow is an id it cannot
//! compare, and a gate that skips what it does not understand is the gate
//! that was green the day the duplicate landed. The two sites in this
//! workspace that cannot be followed are named in `UNREADABLE_ALLOW` with the
//! reason each is safe.
//!
//! # What it does not cover
//!
//! It reads lines, not syntax. An id assembled at run time (`format!`), one
//! held in a `static`, one behind a function call, or a `StatusEntry`
//! constructed by a macro would all be reported as unreadable rather than
//! resolved — which fails loudly and is the intended direction. Ids that
//! differ but mean the same thing are outside its reach entirely; this is a
//! collision check, not a naming review.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use brightfield_workbench::Activity;

// ---------------------------------------------------------------------------
// The allowlist
// ---------------------------------------------------------------------------

/// One id expression the scan locates but cannot follow, and the reason that
/// is safe: `(path suffix, expression as written, why)`.
type Allow = (&'static str, &'static str, &'static str);

/// How many unreadable sites may be excused. Small on purpose — the list is
/// where a scan quietly stops being a gate.
const ALLOW_CAP: usize = 5;

/// The sites whose id the scan cannot follow, each with the argument for why
/// its id is compared anyway.
const UNREADABLE_ALLOW: &[Allow] = &[
    (
        "crates/brightfield-workbench/src/subject.rs",
        "id",
        "RunState::status_entry forwards the id its caller passed. The \
         callers are the declaration sites and the scan reads each of them at \
         the call, so following the parameter here would count the same id \
         twice.",
    ),
    (
        "crates/brightfield-workbench/src/activity.rs",
        "self.id()",
        "Activity::id is a match returning one of the ids in Activity::ALL. \
         This test links brightfield-workbench and feeds those ids into the \
         same comparison from ALL itself, so they are compared without the \
         scan having to read the match.",
    ),
];

// ---------------------------------------------------------------------------
// Reading the source
// ---------------------------------------------------------------------------

/// Which of the two shapes a site was found by. Recorded so the gate can
/// check that both needles still match real production code — a scan whose
/// needle has gone stale reports nothing and looks exactly like a clean tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// A `StatusEntry { … }` construction.
    Construction,
    /// A `…status_entry(<expr>)` call.
    Call,
}

/// One place production code hands a `StatusEntry` an id.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Site {
    /// Workspace-relative path of the file the site is in.
    rel: String,
    /// Workspace-relative path and 1-based line: `crates/…/x.rs:42`.
    at: String,
    /// The id expression exactly as written.
    expr: String,
    /// How it was found.
    via: Shape,
}

/// How far past a `StatusEntry {` the `id` field may sit before the scan gives
/// up and reports. The struct has five fields and rustfmt puts each on its own
/// line, so a construction that has not named its id inside this many lines is
/// not one this scan claims to understand.
const FIELD_SCAN: usize = 12;

/// The argument of a `…status_entry(<expr>)` call on `line`.
///
/// `None` when the line declares the function rather than calling it, when
/// there is no such call, or when the call takes no argument — which is
/// `Activity::status_entry`, whose id comes from `Activity::id`.
///
/// The needle is anchored at the front: a name that merely *ends* in
/// `status_entry` is a different function. `idle_status_entry` and
/// `draw_status_entry` are both in this workspace, and an unanchored needle
/// read their first argument as an id — five findings that were nothing at
/// all, pinned by `a_longer_name_ending_in_status_entry_is_a_different_call`.
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
        // Field shorthand: the id is whatever local named `id` holds.
        let expr = if trimmed == "id," {
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
            via: Shape::Construction,
        };
    }
    Site {
        rel: rel.to_string(),
        at: format!("{rel}:{}", open + 1),
        expr: format!("<no id field within {FIELD_SCAN} lines of the construction>"),
        via: Shape::Construction,
    }
}

/// Every id-supplying site in one file's text.
fn sites(rel: &str, text: &str) -> Vec<Site> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line == "#[cfg(test)]" {
            // A test module opening in column 0 closes on a brace in column 0.
            i += 1;
            while i < lines.len() && lines[i] != "}" {
                i += 1;
            }
            i += 1;
            continue;
        }
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
                via: Shape::Call,
            });
        }
        i += 1;
    }
    out
}

/// The name and value of a `const NAME: &str = "…";` on `line`, when it is one
/// whose value fits on the line.
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
    // A literal continued onto the next line is not one this reads.
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
            if lines[i] == "#[cfg(test)]" {
                i += 1;
                while i < lines.len() && lines[i] != "}" {
                    i += 1;
                }
                i += 1;
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
    // A constant in the site's own file wins; otherwise the name has to mean
    // one thing across the workspace, or the scan is guessing.
    if let Some(value) = declared.get(&site.rel) {
        return Read::Id(value.clone());
    }
    let distinct: BTreeSet<&String> = declared.values().collect();
    match distinct.len() {
        1 => Read::Id((*distinct.iter().next().expect("one value")).clone()),
        _ => Read::Unreadable(format!(
            "`{}` names {} different constants ({}), so this scan cannot say \
             which one the site means",
            site.expr,
            distinct.len(),
            declared
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

/// Whether `allow` excuses an unreadable site.
fn excused(allow: &[Allow], site: &Site) -> bool {
    allow
        .iter()
        .any(|(suffix, expr, _why)| site.rel.ends_with(suffix) && site.expr == *expr)
}

/// Everything wrong with the ids in `files`, one line each.
///
/// `reserved` carries ids that no source line spells out — read from the
/// linked crate — as `(where they came from, the id)`. `allow` excuses sites
/// whose expression the scan cannot follow; an entry that excuses nothing is
/// itself reported, so the list cannot outlive the site it was written for.
fn violations(
    files: &[(String, String)],
    reserved: &[(&str, String)],
    allow: &[Allow],
) -> Vec<String> {
    let consts = constants(files);
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut out = Vec::new();
    let mut excuses_used: BTreeSet<usize> = BTreeSet::new();

    for (rel, text) in files {
        for site in sites(rel, text) {
            match read_id(&site, &consts) {
                Read::Id(id) => found.entry(id).or_default().push(site.at.clone()),
                Read::Unreadable(why) => {
                    if let Some(index) = allow.iter().position(|(suffix, expr, _)| {
                        site.rel.ends_with(suffix) && site.expr == *expr
                    }) {
                        excuses_used.insert(index);
                        continue;
                    }
                    debug_assert!(!excused(allow, &site));
                    out.push(format!("{}: unreadable status-entry id — {why}", site.at));
                }
            }
        }
    }
    for (from, id) in reserved {
        found
            .entry(id.clone())
            .or_default()
            .push((*from).to_string());
    }

    for (id, at) in &found {
        if at.len() > 1 {
            out.push(format!(
                "status-entry id `{id}` is declared {} times — the rail draws \
                 each of them, so one line would appear {} times:\n    {}",
                at.len(),
                at.len(),
                at.join("\n    ")
            ));
        }
    }
    for (index, (suffix, expr, _why)) in allow.iter().enumerate() {
        if !excuses_used.contains(&index) {
            out.push(format!(
                "UNREADABLE_ALLOW entry ({suffix}, {expr}) excused nothing — \
                 the site it was written for is gone, so the entry is too"
            ));
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Walking the workspace
// ---------------------------------------------------------------------------

fn rs_files(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|e| e.expect("dir entry").path())
        .collect::<Vec<_>>();
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

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The fewest id-supplying sites the workspace may hold before the scan is
/// assumed to have stopped reading rather than the rail to have gone quiet.
///
/// The tree this landed on holds 14, of which 12 resolve to an id and 2 are
/// the excused pair in `UNREADABLE_ALLOW`. A floor rather than an equality, so
/// declaring a new rail line is not a test edit — but a needle that stopped
/// matching drops this to nought, not to eleven.
const SITE_FLOOR: usize = 10;

#[test]
fn no_two_places_declare_the_same_status_rail_id() {
    let files = production_sources();
    assert!(
        files.len() > 50,
        "the scan found {} source files, which is too few to have walked the \
         workspace",
        files.len()
    );

    // Both needles still match real code, not only the synthetic source below.
    let all: Vec<Site> = files
        .iter()
        .flat_map(|(rel, text)| sites(rel, text))
        .collect();
    assert!(
        all.len() >= SITE_FLOOR,
        "the scan found {} status-entry ids in the workspace, under the floor \
         of {SITE_FLOOR} — its needles have stopped matching",
        all.len()
    );
    for shape in [Shape::Construction, Shape::Call] {
        assert!(
            all.iter().any(|s| s.via == shape),
            "the scan found no {shape:?} site in the workspace, so that needle \
             is no longer reading anything"
        );
    }

    let reserved: Vec<(&str, String)> = Activity::ALL
        .iter()
        .map(|a| {
            (
                "brightfield_workbench::Activity::ALL (read from the linked crate)",
                a.id().to_string(),
            )
        })
        .collect();

    let found = violations(&files, &reserved, UNREADABLE_ALLOW);
    assert!(
        found.is_empty(),
        "status-rail id violations ({}):\n{}",
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
// The scan proves it can still fail — every run, on synthetic source
// ---------------------------------------------------------------------------
//
// A source scan degrades silently: a rustfmt change, a renamed helper or a
// mistyped needle turns detection off and leaves a green test behind. These
// drive the same `violations` the gate above drives, over text written here,
// so the day the reading breaks is the day these redden.

/// Two panes in one synthetic workspace, declaring one id between them.
const TWO_PANES: &str = r#"
fn describe() -> Subject {
    Subject::new().with_status(StatusEntry {
        id: "run-state",
        side: StatusSide::Trailing,
    })
}
"#;

const SECOND_PANE: &str = r#"
fn describe() -> Subject {
    Subject::new().with_status(state.status_entry("run-state"))
}
"#;

fn synthetic(files: &[(&str, &str)]) -> Vec<String> {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(rel, text)| ((*rel).to_string(), (*text).to_string()))
        .collect();
    violations(&owned, &[], &[])
}

#[test]
fn a_duplicate_across_two_files_is_reported() {
    let found = synthetic(&[
        ("crates/a/src/chart.rs", TWO_PANES),
        ("crates/a/src/grid.rs", SECOND_PANE),
    ]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].contains("`run-state` is declared 2 times")
            && found[0].contains("crates/a/src/chart.rs:4")
            && found[0].contains("crates/a/src/grid.rs:3"),
        "the finding does not name both declarations: {}",
        found[0]
    );
}

#[test]
fn one_declaration_of_an_id_is_not_a_finding() {
    assert!(synthetic(&[("crates/a/src/chart.rs", TWO_PANES)]).is_empty());
}

#[test]
fn a_constant_and_a_literal_of_the_same_id_collide() {
    let by_const = r#"
const RUN_STATE: &str = "run-state";

fn describe() -> Subject {
    Subject::new().with_status(StatusEntry {
        id: RUN_STATE,
        side: StatusSide::Trailing,
    })
}
"#;
    let found = synthetic(&[
        ("crates/a/src/chart.rs", by_const),
        ("crates/a/src/grid.rs", SECOND_PANE),
    ]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].contains("`run-state` is declared 2 times"),
        "a constant id was not followed to its value: {}",
        found[0]
    );
}

#[test]
fn a_duplicate_inside_a_test_module_is_not_a_finding() {
    let with_tests = r#"
fn describe() -> Subject {
    Subject::new().with_status(StatusEntry {
        id: "run-state",
        side: StatusSide::Trailing,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_fixture() {
        let _ = StatusEntry {
            id: "run-state",
            side: StatusSide::Trailing,
        };
    }
}
"#;
    assert!(
        synthetic(&[("crates/a/src/chart.rs", with_tests)]).is_empty(),
        "a fixture in a test module was counted as a declaration"
    );
}

#[test]
fn an_id_the_scan_cannot_follow_is_reported() {
    let computed = r#"
fn describe() -> Subject {
    Subject::new().with_status(StatusEntry {
        id: leak(&format!("pane-{n}")),
        side: StatusSide::Trailing,
    })
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", computed)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].starts_with("crates/a/src/chart.rs:4: unreadable status-entry id"),
        "an id built at run time went unreported: {}",
        found[0]
    );
}

#[test]
fn a_construction_with_no_id_field_is_reported() {
    let headless = r#"
fn describe() -> Subject {
    Subject::new().with_status(StatusEntry {
        side: StatusSide::Trailing,
    })
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", headless)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].contains("no id field"),
        "a construction that names no id went unreported: {}",
        found[0]
    );
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
        "a function whose name merely ends in `status_entry` was read as a \
         declaration"
    );
}

#[test]
fn a_reserved_id_collides_with_a_source_declaration() {
    let clash = r#"
fn describe() -> Subject {
    Subject::new().with_status(StatusEntry {
        id: "activity-file-watch",
        side: StatusSide::Trailing,
    })
}
"#;
    let owned = vec![("crates/a/src/chart.rs".to_string(), clash.to_string())];
    let reserved: Vec<(&str, String)> = Activity::ALL
        .iter()
        .map(|a| ("Activity::ALL", a.id().to_string()))
        .collect();
    let found = violations(&owned, &reserved, &[]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].contains("`activity-file-watch` is declared 2 times")
            && found[0].contains("Activity::ALL"),
        "a pane colliding with an activity id went unreported: {}",
        found[0]
    );
}

#[test]
fn an_allowlist_entry_that_excuses_nothing_is_reported() {
    let owned = vec![(
        "crates/a/src/chart.rs".to_string(),
        TWO_PANES.trim().to_string(),
    )];
    let stale: &[Allow] = &[("crates/a/src/gone.rs", "self.id()", "a site since deleted")];
    let found = violations(&owned, &[], stale);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].contains("excused nothing"),
        "a stale allowlist entry went unreported: {}",
        found[0]
    );
}

#[test]
fn an_allowlist_entry_excuses_only_the_expression_it_names() {
    let computed = r#"
fn describe() -> Subject {
    Subject::new().with_status(StatusEntry {
        id: self.id(),
        side: StatusSide::Trailing,
    })
}
"#;
    let owned = vec![("crates/a/src/activity.rs".to_string(), computed.to_string())];
    let matching: &[Allow] = &[("crates/a/src/activity.rs", "self.id()", "read from ALL")];
    assert!(
        violations(&owned, &[], matching).is_empty(),
        "the entry did not excuse the site it names"
    );

    let other: &[Allow] = &[("crates/a/src/activity.rs", "other.id()", "read from ALL")];
    let found = violations(&owned, &[], other);
    assert_eq!(
        found.len(),
        2,
        "an entry naming a different expression should excuse nothing and be \
         reported stale; got {found:?}"
    );
}
