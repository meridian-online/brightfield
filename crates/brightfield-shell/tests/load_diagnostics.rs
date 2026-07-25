//! Gate: what a spec load could not draw is SAID, in the window.
//!
//! Two mechanisms existed and reached nobody. `brightfield-conformance` — the
//! crate holding the preflight walk that answers "which parts of this spec
//! can we not render" — was a dependency of no application crate. And all
//! four spec-load entry points in `pipeline.rs` moved `.spec` out of their
//! `ParseOutput` and dropped `.warnings` on the floor. A user opening a spec
//! with an unrenderable mark in it got a chart missing that mark and not one
//! word about why.
//!
//! GPU-free throughout: `MeridianApp::headless_with_layout` builds the real
//! window over a real composition without a device, so what the banners say
//! is asserted against the shipping code path rather than a copy of it.

use std::fs;
use std::path::PathBuf;

use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec, compose_spec_str, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// A chart that renders, with one thing in it that cannot.
///
/// `dot` draws; `voronoi` has no lowerer and no renderer, so it is a blocking
/// preflight entry. The `sort` option on the dot is ignored by every lowerer
/// and renderer, so it is an advisory one. Inline data, so it composes from
/// anywhere.
const MIXED: &str = r#"
meta:
  title: Diagnostics fixture
data:
  t:
    - { a: 1, b: 2 }
    - { a: 2, b: 3 }
    - { a: 3, b: 5 }
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
    sort: { y: -x, limit: 10 }
  - mark: voronoi
    data: { from: t }
    x: a
    y: b
"#;

/// The same chart with nothing wrong with it.
const CLEAN: &str = r#"
meta:
  title: Clean fixture
data:
  t:
    - { a: 1, b: 2 }
    - { a: 2, b: 3 }
    - { a: 3, b: 5 }
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
"#;

fn window_over(source: &str) -> MeridianApp {
    let composed = compose_spec_str(source, None).expect("fixture composes");
    MeridianApp::headless_with_layout(Boot::charts(composed), default_layout(), Mode::Light)
}

/// Every banner's title and body, joined — what a reader would see.
fn banner_text(app: &MeridianApp) -> String {
    app.notifications()
        .iter()
        .map(|n| format!("{}\n{}", n.title, n.body.clone().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The blocking entry is named to the user, by its Mosaic wire name and by
/// where it appeared. The wire name matters specifically: it is the string
/// the reader typed into their own file, so it is the one they can search
/// for.
#[test]
fn an_unrenderable_feature_is_named_in_the_window() {
    let app = window_over(MIXED);
    let text = banner_text(&app);
    assert!(
        text.contains("voronoi"),
        "the unrenderable mark must be named in a banner:\n{text}"
    );
    assert!(text.contains("mark"), "…and where it appeared:\n{text}");
}

/// A parse warning reaches a user-visible surface. This is the half that all
/// four load entry points used to discard.
#[test]
fn a_parse_warning_reaches_a_banner() {
    let app = window_over(MIXED);
    let text = banner_text(&app);
    assert!(
        text.contains("sort"),
        "the ignored mark option must reach the window:\n{text}"
    );
}

/// A spec that renders whole says nothing. A channel that speaks on every
/// load is a channel that gets ignored on the load that mattered.
#[test]
fn a_clean_spec_raises_no_banner() {
    let app = window_over(CLEAN);
    assert_eq!(
        app.notifications().len(),
        0,
        "a clean load must be silent: {}",
        banner_text(&app)
    );
    assert!(app.load_diagnostics().is_empty());
}

/// Opening a second document takes the first one's diagnostics down. A banner
/// about a spec the user has closed is worse than no banner.
#[test]
fn opening_a_clean_document_clears_the_previous_ones_diagnostics() {
    let mut app = window_over(MIXED);
    assert!(!app.notifications().is_empty(), "the fixture warns");

    let clean = compose_spec_str(CLEAN, None).expect("clean fixture composes");
    app.open_chart(clean);
    assert_eq!(
        app.notifications().len(),
        0,
        "the previous document's banners must not outlive it: {}",
        banner_text(&app)
    );
}

/// All four spec-load entry points carry the diagnostics. Each is exercised
/// through its real signature, because "the type has a field" is not the
/// property under test — "this entry point fills it" is.
#[test]
fn every_spec_load_entry_point_carries_the_diagnostics() {
    let dir = std::env::temp_dir().join(format!("bf-load-diagnostics-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    let path: PathBuf = dir.join("mixed.yaml");
    fs::write(&path, MIXED).expect("write fixture");
    let path_str = path.to_str().expect("utf-8 path");

    // 1. compose_spec — the path-taking one-shot the capture tiers use.
    let composed = compose_spec(path_str).expect("composes");
    assert!(
        composed
            .diagnostics
            .blocking_names()
            .contains(&"voronoi".to_string()),
        "compose_spec drops its diagnostics"
    );
    assert_eq!(
        composed.diagnostics.source.as_deref(),
        Some("mixed.yaml"),
        "a file-loaded spec cites the file the reader has open"
    );

    // 2. compose_spec_str — the text-taking one the shipped starts use.
    let from_text = compose_spec_str(MIXED, None).expect("composes");
    assert!(
        from_text
            .diagnostics
            .blocking_names()
            .contains(&"voronoi".to_string()),
        "compose_spec_str drops its diagnostics"
    );

    // 3. live_spec — what the window boots a command-line spec through.
    let (mut live, first) = brightfield_shell::pipeline::live_spec(path_str).expect("loads live");
    assert!(
        first
            .diagnostics
            .blocking_names()
            .contains(&"voronoi".to_string()),
        "live_spec drops its diagnostics"
    );

    // 4. LiveDashboard::load_str — the text-taking live loader.
    let live_str = LiveDashboard::load_str(MIXED, None).expect("loads live from text");
    assert!(
        live_str
            .diagnostics()
            .blocking_names()
            .contains(&"voronoi".to_string()),
        "LiveDashboard::load_str drops its diagnostics"
    );

    // And a re-present after the spec has been live does not lose them: the
    // spec did not become renderable because something re-queried.
    let again = live.present().expect("re-presents");
    assert!(
        again
            .diagnostics
            .blocking_names()
            .contains(&"voronoi".to_string()),
        "a re-present must not silence the load's diagnostics"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Every distinct advisory line is in the banner body, not just a count of
/// them. A warning summarised into a number has not been told.
#[test]
fn every_advisory_line_reaches_the_banner_body() {
    let app = window_over(MIXED);
    let diagnostics = app.load_diagnostics();
    let text = banner_text(&app);
    let mut seen: Vec<String> = Vec::new();
    for d in diagnostics.advisory() {
        let line = d.to_string();
        if seen.contains(&line) {
            continue;
        }
        assert!(
            text.contains(&line),
            "advisory line missing from the window: {line}\nbanners:\n{text}"
        );
        seen.push(line);
    }
    assert!(!seen.is_empty(), "the fixture produces advisories");
}
