//! A chart edit written into the chart file's own text.
//!
//! brightfield changes a chart by applying a [`ChartEdit`] to the parsed
//! [`Spec`](brightfield_spec::Spec). [`serialise_spec`](brightfield_spec::serialise_spec)
//! writes a whole spec afresh and keeps no comment, so it cannot put an edit
//! into a file an analyst has read or written in. [`write_chart_edit`] writes
//! the same edit into the text the spec was parsed from, as a change to one
//! line, and the bytes around it — the header comment, the comment on each
//! tile, an analyst's own notes — come back as they were
//! (`a_scale_switch_on_a_generated_tile_adds_one_line_inside_its_plot`).
//!
//! **The splice is arcform's.** [`arc::spec::apply_yaml_edits`] is the one
//! format-preserving YAML writer in the estate, and this module does not edit
//! YAML itself: it maps the edit to a path and a value and hands both over. The path
//! comes from [`plot_route`], which walks the same tree the reducer walks.
//!
//! **What is written.** A [`ChartEdit::SetPlotAttribute`], the edit the scale
//! and normalise switches make. When the plot's mapping carries the key, the
//! value's bytes are replaced and a comment after it on the line stays; when it
//! does not, one `key: value` line is added inside the plot's mapping. The four
//! mark edits are refused by kind until each has a writer of its own.
//!
//! **What is promised.** The text returned parses to exactly the spec
//! [`apply`] makes of the parsed input. That is checked before the text is
//! returned, so a value the splice cannot place the way the parser reads it
//! back is a refusal and not a file. An edit [`apply`] refuses is refused here
//! for the same reason, and an edit that leaves the spec as it was returns the
//! input unchanged.

use std::fmt;

use arc::spec::{apply_yaml_edits, PathPart, SpecEdit};
use brightfield_spec::edit::{apply, plot_route, ChartEdit, RefuseReason};
use brightfield_spec::{parse_spec, serialise_value, Format, SpecValue};

/// Why [`write_chart_edit`] returned no text. Each variant's [`fmt::Display`]
/// is the reason a surface shows.
#[derive(Debug, Clone, PartialEq)]
pub enum ChartTextRefusal {
    /// An edit of a kind that has no text writer yet: the mark edits.
    UnwrittenKind {
        /// The edit's kind, as [`ChartEdit::kind_name`] spells it.
        kind: &'static str,
    },
    /// The text does not parse as a chart, so the plot cannot be found in it.
    Unparsed {
        /// The parser's error.
        detail: String,
    },
    /// The edit names a plot the text does not hold.
    NoSuchPlot {
        /// The plot path the edit names.
        plot: String,
    },
    /// [`apply`] refuses the edit on the parsed text.
    Refused {
        /// The reducer's reason.
        reason: RefuseReason,
    },
    /// The value spells over more than one line, and the edit is written as
    /// one line.
    ValueNotOneLine {
        /// The attribute key.
        key: String,
    },
    /// arcform's splice refused the edit.
    Splice {
        /// arcform's error.
        detail: String,
    },
    /// The spliced text parses to a chart other than the one [`apply`] makes.
    ReadsBackDifferently {
        /// The attribute key the edit wrote.
        key: String,
    },
}

impl fmt::Display for ChartTextRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChartTextRefusal::UnwrittenKind { kind } => write!(
                f,
                "a {kind} edit is not yet written into the chart file's text"
            ),
            ChartTextRefusal::Unparsed { detail } => {
                write!(f, "the chart file does not parse: {detail}")
            }
            ChartTextRefusal::NoSuchPlot { plot } => {
                write!(f, "the chart file holds no plot at {plot}")
            }
            ChartTextRefusal::Refused { reason } => f.write_str(reason.clone().reason()),
            ChartTextRefusal::ValueNotOneLine { key } => {
                write!(f, "the value for {key} does not fit on one line")
            }
            ChartTextRefusal::Splice { detail } => {
                write!(f, "the edit could not be spliced into the text: {detail}")
            }
            ChartTextRefusal::ReadsBackDifferently { key } => write!(
                f,
                "the text written for {key} reads back as a different chart"
            ),
        }
    }
}

impl std::error::Error for ChartTextRefusal {}

/// `text` with `edit` written into it as a change to one line, or why not.
///
/// See the module docs for what is written and what is promised. Nothing is
/// read or written on disk: the caller owns the file.
pub fn write_chart_edit(text: &str, edit: &ChartEdit) -> Result<String, ChartTextRefusal> {
    let ChartEdit::SetPlotAttribute { plot, key, value } = edit else {
        return Err(ChartTextRefusal::UnwrittenKind {
            kind: edit.kind_name(),
        });
    };
    let parsed = parse_spec(text, Format::Yaml)
        .map_err(|e| ChartTextRefusal::Unparsed {
            detail: e.to_string(),
        })?
        .spec;
    let mut edited = parsed.clone();
    apply(&mut edited, edit).map_err(|reason| match reason {
        RefuseReason::PlotNotFound => ChartTextRefusal::NoSuchPlot {
            plot: plot.0.clone(),
        },
        reason => ChartTextRefusal::Refused { reason },
    })?;
    if edited == parsed {
        return Ok(text.to_string());
    }
    let route: Vec<PathPart> = plot_route(&parsed, &plot.0)
        .ok_or_else(|| ChartTextRefusal::NoSuchPlot {
            plot: plot.0.clone(),
        })?
        .into_iter()
        .flat_map(|(concat, index)| [PathPart::from(concat), PathPart::from(index)])
        .collect();

    let spelled = one_line(key, value)?;
    let splice = match current_value(text, &route, key) {
        Some(current) => {
            let mut path = route;
            path.push(PathPart::from(key.as_str()));
            // A bare `key:` has an empty value span right after the colon, and
            // the replacement has to bring its own separating space.
            let value = if current.is_null() {
                format!(" {spelled}")
            } else {
                spelled
            };
            SpecEdit::Replace { path, value }
        }
        None => SpecEdit::Add {
            path: route,
            key: key.clone(),
            value: spelled,
        },
    };
    let written = apply_yaml_edits(text, &[splice]).map_err(|e| ChartTextRefusal::Splice {
        detail: e.to_string(),
    })?;

    let reads_back = parse_spec(&written, Format::Yaml).ok().map(|out| out.spec);
    if reads_back.as_ref() != Some(&edited) {
        return Err(ChartTextRefusal::ReadsBackDifferently { key: key.clone() });
    }
    Ok(written)
}

/// `value` spelled as YAML on one line, the way the whole-spec serialiser
/// spells it.
fn one_line(key: &str, value: &SpecValue) -> Result<String, ChartTextRefusal> {
    let not_one_line = || ChartTextRefusal::ValueNotOneLine {
        key: key.to_string(),
    };
    let spelled = serialise_value(value).map_err(|_| not_one_line())?;
    match spelled.strip_suffix('\n') {
        Some(line) if !line.contains('\n') => Ok(line.to_string()),
        _ => Err(not_one_line()),
    }
}

/// The value `key` holds in the mapping at `route`, read from the text itself
/// rather than from the parsed spec — the spec fills a plot's unset keys from
/// `plotDefaults:`, and whether the line exists is a fact about the text.
fn current_value(text: &str, route: &[PathPart], key: &str) -> Option<serde_yaml::Value> {
    let mut node: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
    for part in route {
        node = match part {
            PathPart::Key(k) => node.get(k.as_str())?.clone(),
            PathPart::Index(i) => node.get(*i)?.clone(),
        };
    }
    node.get(key).cloned()
}
