//! Gate: a mark option key nothing reads is SAID, not swallowed.
//!
//! Brightfield parses the whole Mosaic mark option bag, holds it in the AST,
//! and serialises it back out faithfully. It then renders from a small,
//! nameable subset of it. Every other key is dropped in silence — a spec that
//! asks for `curve: monotone-x` and gets straight segments has been lied to,
//! and until this gate went in only the reader could tell.
//!
//! The two proofs below are vendored, byte-for-byte unmodified upstream
//! Mosaic specs, so what they assert is portability truth rather than a
//! fixture written to match the implementation.

use std::collections::BTreeSet;

use brightfield_spec::{parse_spec, Format, ParseWarning};

/// The `mark:key` pairs a spec's parse reports as unconsumed.
fn unconsumed(source: &str) -> BTreeSet<String> {
    let out = parse_spec(source, Format::Yaml).expect("spec parses");
    out.warnings
        .iter()
        .filter_map(|w| match w {
            ParseWarning::UnconsumedMarkOption { mark, key } => Some(format!("{mark}:{key}")),
            _ => None,
        })
        .collect()
}

/// `sorted-bars.yaml` asks a bar chart to sort by descending value and keep
/// the top ten. Brightfield now does both, so neither half may be named here.
///
/// This test used to assert the opposite — `barX:sort` and `barX:sort.limit`
/// were the two lines it demanded — and inverting it is the point the file's
/// header makes: the capability and the diagnostic move together, or the
/// product lies in one direction or the other. A `sort:` shape brightfield
/// does NOT compute is still reported, by `ParseWarning::UnconsumedSort`;
/// `tests/sort_lift.rs` holds that half.
#[test]
fn sorted_bars_sort_and_limit_are_no_longer_named() {
    let found = unconsumed(include_str!("../vendor/mosaic-specs/yaml/sorted-bars.yaml"));
    assert!(
        !found.contains("barX:sort"),
        "`sort` has a reader and must not be reported as an option nothing \
         reads: {found:?}"
    );
    assert!(
        !found.contains("barX:sort.limit"),
        "the `limit` nested inside it is read by the same lowerer: {found:?}"
    );
}

/// `seattle-temp.yaml` asks for monotone-interpolated areas at a quarter
/// opacity. Brightfield draws straight-edged areas at full opacity.
#[test]
fn seattle_temp_curve_and_fill_opacity_are_named() {
    let found = unconsumed(include_str!(
        "../vendor/mosaic-specs/yaml/seattle-temp.yaml"
    ));
    assert!(
        found.contains("areaY:curve"),
        "the ignored `curve` option must be named: {found:?}"
    );
    assert!(
        found.contains("areaY:fillOpacity"),
        "the ignored `fillOpacity` option must be named: {found:?}"
    );
}

/// The diagnostic names the MARK as well as the key — a dashboard with six
/// marks on it is unreadable otherwise.
#[test]
fn the_diagnostic_names_the_mark_it_came_from() {
    let out = parse_spec(
        include_str!("../vendor/mosaic-specs/yaml/seattle-temp.yaml"),
        Format::Yaml,
    )
    .expect("spec parses");
    let rendered: Vec<String> = out
        .warnings
        .iter()
        .filter(|w| matches!(w, ParseWarning::UnconsumedMarkOption { .. }))
        .map(std::string::ToString::to_string)
        .collect();
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("ruleY") && line.contains("strokeDasharray")),
        "the rule mark's dash pattern is ignored and the line must say which \
         mark asked for it: {rendered:?}"
    );
}

/// Consumed keys stay quiet. A gate that fires on `x`/`y`/`fill` would be
/// noise on literally every spec and would be switched off within a week.
#[test]
fn rendered_channels_raise_nothing() {
    let found = unconsumed(
        "plot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: b\n    fill: c\n    \
         stroke: d\n    size: e\n",
    );
    assert!(
        found.is_empty(),
        "the rendered encoding channels must not warn: {found:?}"
    );
}

/// A mark that does not render at all already says so once. Itemising the
/// options of something that draws nothing is noise, not honesty.
#[test]
fn an_unimplemented_mark_does_not_itemise_its_options() {
    let out = parse_spec(
        "plot:\n  - mark: voronoi\n    data: { from: t }\n    r: 3\n    curve: monotone-x\n",
        Format::Yaml,
    )
    .expect("spec parses");
    assert!(
        out.warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::Unimplemented { .. })),
        "an unimplemented mark still says so"
    );
    assert!(
        !out.warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::UnconsumedMarkOption { .. })),
        "…and does not then list the options of a mark that draws nothing"
    );
}

/// **Absence from the consumed list is the whole cause of the diagnostic**,
/// which is why omitting a key that IS read produces a *false* one.
///
/// One mark, two options, both real knobs on a density mark. `bandwidth` is
/// named in `CONSUMED_MARK_OPTION_KEYS` and stays silent; `curve` is not named
/// and warns. Nothing else about them differs — so a reader who reaches for
/// "when in doubt, leave it off and let it warn" is choosing to tell an author
/// their working instruction has no effect. Verify instead: `rg` the key
/// across the lowerers and the renderer, and name the reader in the list's doc
/// comment.
#[test]
fn list_membership_is_the_only_thing_separating_a_warn_from_a_silence() {
    let found = unconsumed(
        "plot:\n  - mark: densityY\n    data: { from: t }\n    x: a\n    bandwidth: 20\n    \
         curve: monotone-x\n",
    );
    assert!(
        !found.contains("densityY:bandwidth"),
        "`bandwidth` is a listed reader and must stay silent: {found:?}"
    );
    assert!(
        found.contains("densityY:curve"),
        "`curve` has no reader and must warn — the diagnostic follows list \
         membership and nothing else: {found:?}"
    );
}

/// Every shipped example must stay clean. These are the specs the product
/// hands a first-time user, and one of them silently ignoring an instruction
/// is the same defect this gate exists to catch — caught at home instead.
#[test]
fn no_shipped_example_silently_ignores_an_option() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut offenders: Vec<String> = Vec::new();
    let mut seen = 0usize;
    for entry in std::fs::read_dir(&dir).expect("examples dir").flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("yaml") {
            continue;
        }
        seen += 1;
        let source = std::fs::read_to_string(&path).expect("read example");
        let Ok(out) = parse_spec(&source, Format::Yaml) else {
            continue;
        };
        for w in &out.warnings {
            if let ParseWarning::UnconsumedMarkOption { mark, key } = w {
                offenders.push(format!(
                    "{}: {mark}:{key}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
    }
    assert!(seen >= 20, "expected the shipped example set; saw {seen}");
    assert!(
        offenders.is_empty(),
        "shipped examples must not ask for anything brightfield ignores: {offenders:?}"
    );
}
