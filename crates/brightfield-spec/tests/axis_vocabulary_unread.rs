//! **An axis attribute a spec sets and brightfield does not read is named, with
//! the plot that carries it, and an axis attribute brightfield reads is not.**
//!
//! Three lists meet here, and each test pins one join between them.
//!
//! - Which names are axis attributes is Mosaic's published schema's, vendored
//!   in `vendor/mosaic-schema/` and read at build time into
//!   `SCHEMA_AXIS_ATTRIBUTES`. The schema test derives the list from the file
//!   and from an edited copy of it, so the warning is shown to follow the file.
//! - Which of them brightfield reads is `READ_AXIS_ATTRIBUTES`, and the probe
//!   test sets each schema name on a plot and asks the layout resolvers whether
//!   anything they return changed, so the list cannot claim a read the code
//!   does not do, nor miss one it does.
//! - What the vendored corpus carries is found by walking each file's raw YAML,
//!   not the parsed spec, so the expected warnings come from what the author
//!   wrote rather than from the parser being tested.

use std::collections::BTreeMap;
use std::path::PathBuf;

use brightfield_spec::axis_vocabulary::{
    schema_axis_attribute_names, unread_axis_attributes, READ_AXIS_ATTRIBUTES,
    SCHEMA_AXIS_ATTRIBUTES,
};
use brightfield_spec::layout::{
    collect_plot_nodes, plot_label, resolve_axis_ends, resolve_axis_reverse, resolve_axis_titles,
    resolve_fixed_domains, resolve_grid_lines, resolve_plot_insets, resolve_plot_scales,
    resolve_tick_counts, resolve_tick_formats,
};
use brightfield_spec::{parse_spec, parse_spec_path, Format, ParseWarning, PlotNode, Spec};

// ---------------------------------------------------------------------------
// The schema
// ---------------------------------------------------------------------------

fn vendored_schema() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/mosaic-schema/v0.24.2.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?} is not JSON: {e}"))
}

/// **The names are the vendored schema's: removing one from the file stops
/// the warning for it, and leaves every other name's warning standing.**
#[test]
fn removing_a_name_from_the_vendored_schema_stops_its_warning() {
    let mut schema = vendored_schema();
    let from_file =
        schema_axis_attribute_names(&schema).expect("the schema declares plot attributes");
    assert_eq!(
        from_file, SCHEMA_AXIS_ATTRIBUTES,
        "the list the parser warns from is the list the vendored schema declares"
    );
    assert!(
        from_file.iter().any(|n| n == "xTickRotate"),
        "the schema carries `xTickRotate`, the attribute this test removes"
    );

    let names: Vec<&str> = from_file.iter().map(String::as_str).collect();
    assert_eq!(
        unread_axis_attributes(["xTickRotate", "yAxis"], &names),
        ["xTickRotate", "yAxis"],
        "both are schema axis attributes no resolver reads"
    );

    for definition in ["PlotAttributes", "Plot"] {
        schema["definitions"][definition]["properties"]
            .as_object_mut()
            .unwrap_or_else(|| panic!("`definitions.{definition}.properties` is an object"))
            .remove("xTickRotate")
            .unwrap_or_else(|| panic!("`{definition}` declares `xTickRotate`"));
    }
    let edited = schema_axis_attribute_names(&schema).expect("the edited schema still parses");
    let edited: Vec<&str> = edited.iter().map(String::as_str).collect();
    assert_eq!(
        unread_axis_attributes(["xTickRotate", "yAxis"], &edited),
        ["yAxis"],
        "a name the schema no longer declares is not warned about; the others still are"
    );
    let mut expected = names.clone();
    expected.retain(|n| *n != "xTickRotate");
    assert_eq!(edited, expected, "the edit removed one name and no other");
}

/// **The parser warns from the schema's list: a plot that sets every schema
/// axis attribute is told about exactly the ones brightfield does not read.**
#[test]
fn a_plot_setting_every_schema_axis_attribute_is_told_about_each_unread_one() {
    let attrs: String = SCHEMA_AXIS_ATTRIBUTES
        .iter()
        .map(|name| format!("{name}: $p\n"))
        .collect();
    let out = parse_spec(&probe_spec(&attrs), Format::Yaml).expect("the spec parses");
    let mut warned: Vec<&str> = out
        .warnings
        .iter()
        .filter_map(|w| match w {
            ParseWarning::UnreadAxisAttribute { attribute, plot } => {
                assert_eq!(plot.as_deref(), Some("root"), "the one plot is the root");
                Some(attribute.as_str())
            }
            _ => None,
        })
        .collect();
    warned.sort_unstable();
    let mut expected: Vec<&str> = SCHEMA_AXIS_ATTRIBUTES
        .iter()
        .copied()
        .filter(|n| !READ_AXIS_ATTRIBUTES.contains(n))
        .collect();
    expected.sort_unstable();
    assert_eq!(warned, expected);
    assert_eq!(
        expected.len() + READ_AXIS_ATTRIBUTES.len(),
        SCHEMA_AXIS_ATTRIBUTES.len(),
        "each read name is a schema name"
    );
}

/// **A facet axis's attribute is an axis attribute when the schema declares it:
/// every `fx…` and `fy…` name the vendored schema carries is on the list, and a
/// name that only starts as one does is not.**
#[test]
fn a_facet_axis_attribute_the_schema_declares_is_an_axis_attribute() {
    let schema = vendored_schema();
    let declared: Vec<String> = schema["definitions"]["PlotAttributes"]["properties"]
        .as_object()
        .expect("the schema declares plot attributes")
        .keys()
        .filter(|name| {
            let mut chars = name.chars();
            chars.next() == Some('f')
                && matches!(chars.next(), Some('x' | 'y'))
                && chars.next().is_some_and(|c| c.is_ascii_uppercase())
        })
        .cloned()
        .collect();
    assert!(
        declared.iter().any(|n| n == "fxLabel") && declared.iter().any(|n| n == "fyTickFormat"),
        "the schema declares the two facet names this test relies on: {declared:?}"
    );
    for name in &declared {
        assert!(
            SCHEMA_AXIS_ATTRIBUTES.contains(&name.as_str()),
            "the schema declares `{name}`, a facet axis's, and the list leaves it out"
        );
        assert!(
            !READ_AXIS_ATTRIBUTES.contains(&name.as_str()),
            "no resolver reads `{name}`"
        );
    }
    // The names that only start the way a facet axis's does, and the ones that
    // are no axis's at all.
    for name in [
        "facetGrid",
        "facetLabel",
        "facetMargin",
        "fxyDomain",
        "xyDomain",
        "fx",
        "fxlabel",
        "ffxLabel",
        "fooBar",
    ] {
        assert!(
            !SCHEMA_AXIS_ATTRIBUTES.contains(&name),
            "`{name}` is no axis attribute"
        );
    }
}

/// **A plot that sets a facet-axis attribute the schema declares is told about
/// it, a `null` included, as an `x` or `y` name set to `null` is; a name that
/// starts as a facet axis's does and is not in the schema is not.**
#[test]
fn a_facet_axis_attribute_is_named_whatever_it_is_set_to_and_an_undeclared_name_is_not() {
    let warned = |attrs: &str| -> Vec<String> {
        let out = parse_spec(&probe_spec(attrs), Format::Yaml).expect("the spec parses");
        out.warnings
            .iter()
            .filter_map(|w| match w {
                ParseWarning::UnreadAxisAttribute { attribute, plot } => {
                    assert_eq!(plot.as_deref(), Some("root"), "the one plot is the root");
                    Some(attribute.clone())
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(warned("fxLabel: Region\n"), ["fxLabel"]);
    assert_eq!(warned("fyTickFormat: '%b'\n"), ["fyTickFormat"]);
    assert_eq!(
        warned("fxLabel: null\n"),
        ["fxLabel"],
        "a `null` is named, as `yAxis: null` is"
    );
    assert_eq!(warned("yAxis: null\n"), ["yAxis"]);
    assert!(
        warned("fxFlavour: 1\nfyBogus: 1\nfacetLabel: x\n").is_empty(),
        "a name the schema does not declare is not an axis attribute, however it starts"
    );
}

// ---------------------------------------------------------------------------
// What the layout reads
// ---------------------------------------------------------------------------

/// One dot on a bare plot. `$p` is declared so a probe may lift a param.
fn probe_spec(attrs: &str) -> String {
    format!(
        "params:\n  p: 1\ndata:\n  t:\n    - {{ a: 1, b: 2 }}\nplot:\n  - {{ mark: dot, data: {{ from: t }}, x: a, y: b }}\n{attrs}"
    )
}

/// Everything the layout resolvers make of one plot's attributes.
fn resolved(plot: &PlotNode) -> String {
    format!(
        "{:?}",
        (
            resolve_plot_insets(plot),
            resolve_axis_titles(plot),
            resolve_fixed_domains(plot),
            resolve_tick_counts(plot),
            resolve_tick_formats(plot),
            resolve_grid_lines(plot),
            resolve_axis_ends(plot),
            resolve_axis_reverse(plot),
            resolve_plot_scales(plot),
        )
    )
}

fn root_plot(spec: &Spec) -> &PlotNode {
    let plots = collect_plot_nodes(spec);
    assert_eq!(plots.len(), 1, "the probe spec has one plot");
    plots[0].1
}

/// One value of each shape an axis attribute takes, so a resolver that reads a
/// key only in one shape (`xDomain: Fixed`, `xScale: log`) is still seen.
const PROBE_VALUES: &[&str] = &[
    "true",
    "false",
    "7",
    "0.5",
    "Fixed",
    "log",
    "'.2f'",
    "'An axis'",
    "[0, 10]",
    "null",
    "''",
];

/// **`READ_AXIS_ATTRIBUTES` is what the layout resolvers read: a name on it
/// changes what a resolver returns for at least one probe value, and a schema
/// name off it leaves each resolver's result as it was for each probe value.**
#[test]
fn the_read_list_is_what_the_layout_resolvers_read() {
    let base = parse_spec(&probe_spec(""), Format::Yaml).expect("the bare probe parses");
    let unset = resolved(root_plot(&base.spec));

    let mut wrong = Vec::new();
    for name in SCHEMA_AXIS_ATTRIBUTES {
        let changes = PROBE_VALUES.iter().any(|value| {
            parse_spec(&probe_spec(&format!("{name}: {value}\n")), Format::Yaml)
                .is_ok_and(|out| resolved(root_plot(&out.spec)) != unset)
        });
        let listed = READ_AXIS_ATTRIBUTES.contains(name);
        if changes != listed {
            wrong.push(format!(
                "{name}: {} by a resolver, {} READ_AXIS_ATTRIBUTES",
                if changes { "read" } else { "not read" },
                if listed { "on" } else { "off" }
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the read list disagrees with the resolvers:\n{}",
        wrong.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

fn corpus() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/mosaic-specs/yaml");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .collect();
    files.sort();
    files
}

/// The root keys that are not a component: the walker reads them as blocks.
const ROOT_BLOCKS: &[&str] = &["meta", "data", "params", "config", "plotDefaults"];

/// Each plot in a raw document, with its component path and the keys it sets
/// itself, walked in the walker's discriminator order: `plot`, then `vconcat`,
/// then `hconcat`.
fn raw_plots(component: &serde_yaml::Mapping, path: &str, out: &mut Vec<(String, Vec<String>)>) {
    let key = |k: &str| component.get(serde_yaml::Value::String(k.into()));
    if key("plot").is_some() {
        let keys = component
            .keys()
            .filter_map(serde_yaml::Value::as_str)
            .filter(|k| *k != "plot")
            .map(str::to_string)
            .collect();
        out.push((path.to_string(), keys));
        return;
    }
    for kind in ["vconcat", "hconcat"] {
        if let Some(serde_yaml::Value::Sequence(items)) = key(kind) {
            for (i, item) in items.iter().enumerate() {
                if let serde_yaml::Value::Mapping(m) = item {
                    raw_plots(m, &format!("{path}/{kind}[{i}]"), out);
                }
            }
            return;
        }
    }
}

/// The distinct axis attributes the vendored corpus sets that brightfield does
/// not read, as they stood when this was written. A vendor bump, a schema bump
/// or a resolver that learns one of them changes this list in the same edit; a
/// walk that read no plot at all fails on it rather than passing over nothing.
const UNREAD_IN_CORPUS: &[&str] = &[
    "fxDomain",
    "fxLabel",
    "fyDomain",
    "fyLabel",
    "xAxis",
    "xLabelAnchor",
    "xLine",
    "xTickSize",
    "yAxis",
    "yLabelAnchor",
    "yLine",
];

/// **Across the vendored corpus, a warning names each axis attribute a spec
/// carries that brightfield does not read, with the plot that carries it, and
/// none names an attribute brightfield reads.** Prints the attributes warned
/// about and the specs that carry them.
#[test]
fn each_unread_axis_attribute_in_the_corpus_is_named_and_no_read_one_is() {
    let mut carried_read = 0usize;
    let mut table: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let raw: serde_yaml::Value =
            serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let serde_yaml::Value::Mapping(root) = raw else {
            panic!("{name}: the document is not a mapping");
        };
        let Ok(parsed) = parse_spec_path(&path) else {
            continue; // corpus_totality is the gate for a file that does not parse
        };

        // What the author wrote: each plot's own keys, then `plotDefaults:`.
        let mut component = root.clone();
        for block in ROOT_BLOCKS {
            component.remove(serde_yaml::Value::String((*block).into()));
        }
        let mut plots = Vec::new();
        raw_plots(&component, "root", &mut plots);
        let labels: BTreeMap<String, String> = collect_plot_nodes(&parsed.spec)
            .into_iter()
            .map(|(at, plot)| (at.clone(), plot_label(&at, plot)))
            .collect();
        let mut carriers: Vec<(Option<String>, Vec<String>)> = plots
            .into_iter()
            .map(|(at, keys)| {
                let label = labels
                    .get(&at)
                    .unwrap_or_else(|| panic!("{name}: the parser placed no plot at {at}"));
                (Some(label.clone()), keys)
            })
            .collect();
        if let Some(serde_yaml::Value::Mapping(defaults)) =
            root.get(serde_yaml::Value::String("plotDefaults".into()))
        {
            let keys = defaults
                .keys()
                .filter_map(serde_yaml::Value::as_str)
                .map(str::to_string);
            carriers.push((None, keys.collect()));
        }

        let mut expected: Vec<(Option<String>, String)> = Vec::new();
        for (plot, keys) in &carriers {
            for key in keys
                .iter()
                .filter(|k| SCHEMA_AXIS_ATTRIBUTES.contains(&k.as_str()))
            {
                if READ_AXIS_ATTRIBUTES.contains(&key.as_str()) {
                    carried_read += 1;
                } else {
                    expected.push((plot.clone(), key.clone()));
                    let specs = table.entry(key.clone()).or_default();
                    if !specs.contains(&name) {
                        specs.push(name.clone());
                    }
                }
            }
        }

        let mut warned: Vec<(Option<String>, String)> = parsed
            .warnings
            .iter()
            .filter_map(|w| match w {
                ParseWarning::UnreadAxisAttribute { attribute, plot } => {
                    Some((plot.clone(), attribute.clone()))
                }
                _ => None,
            })
            .collect();
        for (_, attribute) in &warned {
            assert!(
                !READ_AXIS_ATTRIBUTES.contains(&attribute.as_str()),
                "{name}: `{attribute}` is read, and was warned about"
            );
        }
        expected.sort();
        warned.sort();
        assert_eq!(
            warned, expected,
            "{name}: the warnings are the unread axis attributes it sets"
        );
    }

    println!("axis attributes the vendored corpus sets and brightfield does not read:");
    for (attribute, specs) in &table {
        println!("  {attribute}: {}", specs.join(", "));
    }
    assert!(
        carried_read > 0,
        "the corpus sets axis attributes brightfield reads, so their silence is tested"
    );
    let found: Vec<&str> = table.keys().map(String::as_str).collect();
    assert_eq!(found, UNREAD_IN_CORPUS);
}

// ---------------------------------------------------------------------------
// The corpus's axis instructions that an axis may drop
// ---------------------------------------------------------------------------

/// The axis instructions an axis may take none of, as a composition names them
/// when it lands on the wrong kind of axis: the ends of an axis, a tick count,
/// and a reversal.
const WATCHED: &[&str] = &[
    "xZero", "xNice", "xTicks", "xReverse", "yZero", "yNice", "yTicks", "yReverse",
];

/// Each plot in a raw document with its component path and every key it sets
/// itself, value and all: `raw_plots` with the values kept.
fn raw_plot_attributes(
    component: &serde_yaml::Mapping,
    path: &str,
    out: &mut Vec<(String, BTreeMap<String, serde_yaml::Value>)>,
) {
    let key = |k: &str| component.get(serde_yaml::Value::String(k.into()));
    if key("plot").is_some() {
        let attrs = component
            .iter()
            .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.clone())))
            .filter(|(k, _)| k != "plot")
            .collect();
        out.push((path.to_string(), attrs));
        return;
    }
    for kind in ["vconcat", "hconcat"] {
        if let Some(serde_yaml::Value::Sequence(items)) = key(kind) {
            for (i, item) in items.iter().enumerate() {
                if let serde_yaml::Value::Mapping(m) = item {
                    raw_plot_attributes(m, &format!("{path}/{kind}[{i}]"), out);
                }
            }
            return;
        }
    }
}

/// Whether what the plot writes of itself makes `key` land where it does
/// nothing whatever the data is: a reversal on a plot with a projection, or an
/// end or a tick count on an axis whose `xScale` / `yScale` the spec writes as
/// a log, a symlog or a band, or, for an end, a time. What a column's type makes
/// of an axis is a composition's to say, and not decidable here.
fn inert_by_what_the_plot_writes(key: &str, attrs: &BTreeMap<String, serde_yaml::Value>) -> bool {
    let (axis, instruction) = key.split_at(1);
    if instruction == "Reverse" {
        return attrs.contains_key("projectionType");
    }
    let scale = attrs
        .get(&format!("{axis}Scale"))
        .and_then(serde_yaml::Value::as_str);
    matches!(
        (instruction, scale),
        (
            "Ticks",
            Some("log" | "symlog" | "band" | "ordinal" | "point")
        ) | (
            "Zero" | "Nice",
            Some("log" | "symlog" | "band" | "ordinal" | "point" | "time" | "utc")
        )
    )
}

/// The plots of the vendored and the curated corpus that set one of
/// [`WATCHED`], as `spec  plot  key`, as they stood when this was written; a key
/// a plot inherits is named once, under `plotDefaults`. A vendor bump changes
/// this list in the same edit.
///
/// What each lands on is a column's type, and the corpus's data is not in the
/// repository, so a composition's verdict on these is not tested here. Two are
/// worth reading: `line-multi-series` and `region-tests` set `xTicks` on a
/// `date` read from a Parquet file. A `TIMESTAMP` column takes a time axis,
/// which follows the count, and a `DATE` column takes a band, which does not
/// and which the banner would then name. The rest sit on numeric columns.
const WATCHED_IN_CORPUS: &[(&str, &str, &str)] = &[
    ("flights-density.yaml", "root/vconcat[1]", "xZero"),
    ("gaia.yaml", "root/hconcat[2]", "yReverse"),
    ("line-density.yaml", "root/vconcat[2]", "yNice"),
    ("line-density.yaml", "root/vconcat[3]", "yNice"),
    ("line-multi-series.yaml", "root", "xTicks"),
    ("population-arrows.yaml", "root/vconcat[1]", "yTicks"),
    ("region-tests.yaml", "root/vconcat[0]", "xTicks"),
    ("splom.yaml", "plotDefaults", "xTicks"),
    ("splom.yaml", "plotDefaults", "yTicks"),
];

/// **Across the vendored corpus and the curated corpus, no plot sets a watched
/// key on an axis its own text makes drop it, so each warning a composition
/// raises over them is one the data's column types decide.** The curated corpus
/// sets none. Prints each plot that sets one, by spec, plot and key.
#[test]
fn each_watched_axis_instruction_in_the_corpus_is_listed_and_none_is_inert_by_its_own_text() {
    let curated = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-conformance/vendor/curated/yaml");
    let mut files = corpus();
    files.extend(
        std::fs::read_dir(&curated)
            .unwrap_or_else(|e| panic!("read {curated:?}: {e}"))
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|e| e.to_str()) == Some("yaml")
                    && !path.to_string_lossy().ends_with(".expected.yaml")
            }),
    );

    let mut found: Vec<(String, String, String)> = Vec::new();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let raw: serde_yaml::Value =
            serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let serde_yaml::Value::Mapping(root) = raw else {
            panic!("{name}: the document is not a mapping");
        };
        let mut component = root.clone();
        for block in ROOT_BLOCKS {
            component.remove(serde_yaml::Value::String((*block).into()));
        }
        let defaults: BTreeMap<String, serde_yaml::Value> =
            match root.get(serde_yaml::Value::String("plotDefaults".into())) {
                Some(serde_yaml::Value::Mapping(m)) => m
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.clone())))
                    .collect(),
                _ => BTreeMap::new(),
            };
        let mut plots = Vec::new();
        raw_plot_attributes(&component, "root", &mut plots);
        for (at, mut attrs) in plots {
            let own: Vec<String> = attrs.keys().cloned().collect();
            for (key, value) in &defaults {
                attrs.entry(key.clone()).or_insert_with(|| value.clone());
            }
            for key in WATCHED.iter().filter(|k| attrs.contains_key(**k)) {
                assert!(
                    !inert_by_what_the_plot_writes(key, &attrs),
                    "{name} {at}: `{key}` is set where the spec's own text makes it do nothing"
                );
                // A key a plot inherits is named once, under `plotDefaults`.
                if own.iter().any(|k| k == key) {
                    found.push((name.clone(), at.clone(), (*key).to_string()));
                }
            }
        }
        for key in WATCHED.iter().filter(|k| defaults.contains_key(**k)) {
            found.push((name.clone(), "plotDefaults".to_string(), (*key).to_string()));
        }
    }

    println!("watched axis instructions the corpora set, by spec, plot and key:");
    for (spec, plot, key) in &found {
        println!("  {spec}  {plot}  {key}");
    }
    let found: Vec<(&str, &str, &str)> = found
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    assert_eq!(found, WATCHED_IN_CORPUS);
}
