//! **The date format brightfield reads prints what d3-time-format prints.**
//!
//! A Mosaic spec's `xTickFormat` / `yTickFormat` on a date axis is a
//! d3-time-format specifier, and d3-time-format is the authority for what one
//! prints. `d3_time_format_vectors/table.rs` holds what the real d3-time-format
//! 4.1.0 prints, in UTC, for every directive it reads, each padding modifier on the
//! directives that pad, and a few whole specifiers, at instants chosen for where a
//! calendar port goes wrong. `d3_time_format_vectors/gen-vectors.mjs` generated it,
//! and these tests hold the Rust port to it, string for string.

use brightfield_spec::date_format::DateFormat;

#[path = "d3_time_format_vectors/table.rs"]
mod table;

/// Every `(specifier, instant)` the table holds prints as d3-time-format prints
/// it. Mismatches are collected so a failure names every one, not the first.
#[test]
fn format_prints_what_d3_time_format_prints() {
    let mut wrong = Vec::new();
    for (spec, ms, expected) in table::DATE_VECTORS {
        let Ok(format) = DateFormat::parse(spec) else {
            wrong.push(format!("`{spec}` did not parse"));
            continue;
        };
        let got = format.format(ms * 1_000);
        if got != *expected {
            wrong.push(format!(
                "format({spec:?})({ms}) = {got:?}, d3-time-format prints {expected:?}"
            ));
        }
    }
    assert!(
        table::DATE_VECTORS.len() > 1_000,
        "the table shrank to {} rows; a vector test over a table that is empty proves nothing",
        table::DATE_VECTORS.len()
    );
    assert!(
        wrong.is_empty(),
        "{} of {} vectors differ:\n{}",
        wrong.len(),
        table::DATE_VECTORS.len(),
        wrong.join("\n")
    );
}

/// The table covers every directive this build reads, so a directive added to
/// the port without a vector, or one the table forgot, is a failure here and
/// not a silent gap.
#[test]
fn the_table_holds_every_directive_the_port_reads() {
    for directive in "aAbBcdefgGHIjLmMpqQsSuUVwWxXyYZ%".chars() {
        let spec = format!("%{directive}");
        assert!(
            table::DATE_VECTORS.iter().any(|(s, _, _)| *s == spec),
            "no vector holds `{spec}`"
        );
    }
}
