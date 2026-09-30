//! **Each tick format the vendored corpus carries is one this build reads or
//! defers, and none is warned about.**
//!
//! The corpus is the vendored Mosaic examples, so the `xTickFormat` and
//! `yTickFormat` values in it are ones a real author wrote. This walks the plots of each
//! and asks the one judge the parser and the renderer share, [`read_tick_format`]:
//! a number format is read as one, the date format the corpus also carries
//! (`%b`) is read as one, and the parse warns of no tick format in any file.
//!
//! The walk finds its own formats rather than naming them, but it also names
//! the four number formats and the one date format the corpus held when this
//! was written, so a vendor bump that dropped a key, or a walk that read no
//! plot at all, would fail here and not pass over nothing.

use std::path::PathBuf;

use brightfield_spec::layout::{collect_plot_nodes, read_tick_format, AxisFormat, TickFormatReading};
use brightfield_spec::{parse_spec_path, ParseWarning, SpecValue};

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

#[test]
fn every_tick_format_in_the_corpus_is_read_or_deferred_and_none_is_warned_about() {
    let mut numbers: Vec<String> = Vec::new();
    let mut dates: Vec<String> = Vec::new();

    for path in corpus() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let Ok(parsed) = parse_spec_path(&path) else {
            continue; // corpus_totality is the gate for a file that does not parse
        };

        let warned: Vec<&ParseWarning> = parsed
            .warnings
            .iter()
            .filter(|w| {
                matches!(
                    w,
                    ParseWarning::InvalidTickFormat { .. }
                        | ParseWarning::UnreadDateDirective { .. }
                )
            })
            .collect();
        assert!(
            warned.is_empty(),
            "{name} is a vendored spec and its tick formats must not warn: {warned:?}"
        );

        for (at, plot) in collect_plot_nodes(&parsed.spec) {
            for key in ["xTickFormat", "yTickFormat"] {
                let Some(value) = plot.attributes.get(key) else {
                    continue;
                };
                let SpecValue::String(text) = value else {
                    panic!("{name}::{at} {key} is not a string: {value:?}");
                };
                match read_tick_format(value) {
                    TickFormatReading::Format(AxisFormat::Number(_)) => numbers.push(text.clone()),
                    TickFormatReading::Format(AxisFormat::Date(_)) => dates.push(text.clone()),
                    other => panic!("{name}::{at} {key}: `{text}` is not read: {other:?}"),
                }
            }
        }
    }

    numbers.sort();
    numbers.dedup();
    dates.sort();
    dates.dedup();
    assert_eq!(
        numbers,
        ["%", "+f", "d", "s"],
        "the number formats the corpus carries, each read"
    );
    assert_eq!(
        dates,
        ["%b"],
        "the date format the corpus carries, read and not warned about"
    );
}
