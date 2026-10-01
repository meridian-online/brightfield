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
//! **What is written.** Four of [`ChartEdit`]'s seven kinds, each as a change
//! to the lines it names and no other:
//!
//! - A [`ChartEdit::SetPlotAttribute`], the edit the scale and normalise
//!   switches make. When the plot's mapping carries the key, the value's bytes
//!   are replaced and a comment after it on the line stays; when it does not,
//!   one `key: value` line is added after the plot's last attribute.
//! - A [`ChartEdit::SetChannel`], the edit the shelf makes on each layer it
//!   moves. When the mark carries the channel as a column name, the name inside
//!   the value is rewritten, so the value keeps the quotes it was written in
//!   (`x: 'longitude'` becomes `x: 'median_income'`); a value the rewrite
//!   cannot place, or one that is not a column name, is replaced in the
//!   whole-spec serialiser's spelling. A mark without the channel gains one
//!   line after its last key.
//! - A [`ChartEdit::AddColourLegend`], the edit the shelf makes when a column
//!   goes on colour. One `- legend: color` line is added after the last item of
//!   the plot's list, indented as the list's own items are, and above a
//!   comment that closes the list. A plot that already holds the item
//!   is the spec as it was, so the text comes back unchanged. The indent is
//!   read from arcform: a throwaway key is added to the last item, the line it
//!   lands on says how far in the item's keys sit, and the item goes two
//!   columns short of that. A list whose items are written in flow style gives
//!   no such line, and is refused; so is a list whose item the `- ` is not two
//!   columns wide for, because the text then reads back as another chart.
//! - A [`ChartEdit::RemovePlotAttribute`], the edit that takes a map's
//!   projection out. The key's line is taken out. By arcform's rule of comment
//!   ownership a comment flush above the line, indented no deeper than it, is
//!   that line's header and goes with it; every other comment stays
//!   (`a_comment_flush_above_a_removed_line_goes_with_it_and_the_rest_stay`).
//!
//! Change mark type, add mark and remove mark are refused by kind until each
//! has a writer of its own.
//!
//! **One gesture is several edits, written one at a time.** The shelf's edit
//! on the generated map is a set channel on each of its two layers and a
//! removal of its projection, and Save places each into the text the one
//! before it left. An added line lands after the last line of its mapping, so
//! an attribute taken out and put back returns to its place when it was the
//! mapping's last; the generator writes the map's projection last for that
//! reason, and `putting_longitude_back_on_x_gives_the_text_the_generator_wrote`
//! holds the round trip.
//!
//! **What is promised.** The text returned parses to exactly the spec
//! [`apply_for_fresh_load`] makes of the parsed input. That is checked before
//! the text is returned, so a value the splice cannot place the way the parser
//! reads it back is a refusal and not a file. The reducer is the fresh-load
//! one and not [`apply`](brightfield_spec::edit::apply) because Save writes the
//! edits the page was already drawn with, and a page loaded afresh draws an
//! axis title that changes: [`apply`](brightfield_spec::edit::apply)'s chrome
//! refusals belong to a reload from disk, which this text is not. An edit with
//! no target is refused here for the reducer's reason, and an edit that leaves
//! the spec as it was returns the input unchanged.

use std::fmt;

use arc::spec::{apply_yaml_edits, PathPart, SpecEdit};
use brightfield_spec::edit::{
    apply_for_fresh_load, mark_item_index, plot_at_path, plot_route, ChartEdit, RefuseReason,
};
use brightfield_spec::{parse_spec, serialise_value, Format, SpecValue};

/// Why [`write_chart_edit`] returned no text. Each variant's [`fmt::Display`]
/// is the reason a surface shows.
#[derive(Debug, Clone, PartialEq)]
pub enum ChartTextRefusal {
    /// An edit of a kind that has no text writer yet: change mark type, add
    /// mark and remove mark.
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
    /// [`apply_for_fresh_load`] refuses the edit on the parsed text.
    Refused {
        /// The reducer's reason.
        reason: RefuseReason,
    },
    /// The value spells over more than one line, and the edit is written as
    /// one line.
    ValueNotOneLine {
        /// The attribute or channel key.
        key: String,
    },
    /// The plot takes the attribute a removal names from `plotDefaults:`, so
    /// the plot's own mapping has no line to take out.
    Inherited {
        /// The attribute key.
        key: String,
    },
    /// arcform's splice refused the edit.
    Splice {
        /// arcform's error.
        detail: String,
    },
    /// The spliced text parses to a chart other than the one
    /// [`apply_for_fresh_load`] makes.
    ReadsBackDifferently {
        /// The attribute or channel key the edit wrote.
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
            ChartTextRefusal::Inherited { key } => write!(
                f,
                "the plot takes {key} from plotDefaults:, so it has no line of its own to take out"
            ),
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

/// `text` with `edit` written into it as a change to the lines it names, or
/// why not.
///
/// See the module docs for what is written and what is promised. Nothing is
/// read or written on disk: the caller owns the file.
pub fn write_chart_edit(text: &str, edit: &ChartEdit) -> Result<String, ChartTextRefusal> {
    let (plot, key) = match edit {
        ChartEdit::SetPlotAttribute { plot, key, .. }
        | ChartEdit::RemovePlotAttribute { plot, key } => (plot, key.as_str()),
        ChartEdit::SetChannel { plot, channel, .. } => (plot, channel.as_str()),
        ChartEdit::AddColourLegend { plot } => (plot, LEGEND_KEY),
        ChartEdit::ChangeMarkType { .. }
        | ChartEdit::AddMark { .. }
        | ChartEdit::RemoveMark { .. } => {
            return Err(ChartTextRefusal::UnwrittenKind {
                kind: edit.kind_name(),
            })
        }
    };
    let parsed = parse_spec(text, Format::Yaml)
        .map_err(|e| ChartTextRefusal::Unparsed {
            detail: e.to_string(),
        })?
        .spec;
    let mut edited = parsed.clone();
    apply_for_fresh_load(&mut edited, edit).map_err(|reason| match reason {
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

    // The splices to try, in order; the first whose text reads back as the
    // edited spec is the one written.
    let splices = match edit {
        ChartEdit::SetPlotAttribute { key, value, .. } => {
            let spelled = one_line(key, value)?;
            vec![set_key(text, route, key, spelled)]
        }
        ChartEdit::RemovePlotAttribute { key, .. } => {
            if current_value(text, &route, key).is_none() {
                return Err(ChartTextRefusal::Inherited { key: key.clone() });
            }
            let mut path = route;
            path.push(PathPart::from(key.as_str()));
            vec![SpecEdit::Delete { path }]
        }
        ChartEdit::SetChannel {
            mark_ordinal,
            channel,
            column,
            ..
        } => {
            let index = plot_at_path(&parsed, &plot.0)
                .and_then(|p| mark_item_index(p, *mark_ordinal))
                .ok_or(ChartTextRefusal::Refused {
                    reason: RefuseReason::NoSuchMark,
                })?;
            let mut mark = route;
            mark.extend([PathPart::from("plot"), PathPart::from(index)]);
            let spelled = one_line(channel, &SpecValue::String(column.clone()))?;
            let mut splices = Vec::with_capacity(2);
            // The column's name rewritten inside the value keeps the value's
            // quotes; the serialiser's spelling is what a value the rewrite
            // cannot place falls back to.
            if let Some(serde_yaml::Value::String(from)) = current_value(text, &mark, channel) {
                let mut path = mark.clone();
                path.push(PathPart::from(channel.as_str()));
                splices.push(SpecEdit::RewriteFragment {
                    path,
                    from,
                    to: column.clone(),
                });
            }
            splices.push(set_key(text, mark, channel, spelled));
            splices
        }
        ChartEdit::AddColourLegend { .. } => {
            let last = plot_at_path(&parsed, &plot.0)
                .and_then(|p| p.items.len().checked_sub(1))
                .ok_or_else(|| ChartTextRefusal::Splice {
                    detail: "the plot's list has no item to place a legend after".to_string(),
                })?;
            let mut list = route;
            list.push(PathPart::from("plot"));
            let indent = item_indent(text, &list, last)?;
            vec![SpecEdit::Append {
                path: list,
                item: format!("{indent}- {COLOUR_LEGEND_ITEM}"),
            }]
        }
        ChartEdit::ChangeMarkType { .. }
        | ChartEdit::AddMark { .. }
        | ChartEdit::RemoveMark { .. } => {
            return Err(ChartTextRefusal::UnwrittenKind {
                kind: edit.kind_name(),
            })
        }
    };

    let mut refusal = ChartTextRefusal::ReadsBackDifferently {
        key: key.to_string(),
    };
    for splice in splices {
        match apply_yaml_edits(text, &[splice]) {
            Ok(written) => {
                let reads_back = parse_spec(&written, Format::Yaml).ok().map(|out| out.spec);
                if reads_back.as_ref() == Some(&edited) {
                    return Ok(written);
                }
                refusal = ChartTextRefusal::ReadsBackDifferently {
                    key: key.to_string(),
                };
            }
            Err(e) => {
                refusal = ChartTextRefusal::Splice {
                    detail: e.to_string(),
                };
            }
        }
    }
    Err(refusal)
}

/// The key a colour legend item is written under, the one a refusal names.
const LEGEND_KEY: &str = "legend";

/// The colour legend's item, as it is written after the `- `.
const COLOUR_LEGEND_ITEM: &str = "legend: color";

/// The key [`item_indent`] adds to an item to see where arcform puts it.
const INDENT_PROBE_KEY: &str = "indent_probe";

/// The leading spaces the items of the block list at `list` are written with,
/// where `last` is the index of the list's last item.
///
/// arcform indents a key it adds to a mapping like that mapping's other keys,
/// and for an item written `- mark: dot` those keys sit two columns past the
/// item's own `- `. So a probe key is added to the last item, the line it lands
/// on is read, and the probe is thrown away with the text it was added to.
/// arcform's own reader of this, `sequence_item_indent`, is not exported.
fn item_indent(text: &str, list: &[PathPart], last: usize) -> Result<String, ChartTextRefusal> {
    let mut item = list.to_vec();
    item.push(PathPart::from(last));
    let probed = apply_yaml_edits(
        text,
        &[SpecEdit::Add {
            path: item,
            key: INDENT_PROBE_KEY.to_string(),
            value: "0".to_string(),
        }],
    )
    .map_err(|e| ChartTextRefusal::Splice {
        detail: e.to_string(),
    })?;
    probed
        .lines()
        .find(|line| line.trim_start().starts_with(INDENT_PROBE_KEY))
        .and_then(|line| {
            let keys = line.len() - line.trim_start().len();
            keys.checked_sub(2).map(|items| " ".repeat(items))
        })
        .ok_or_else(|| ChartTextRefusal::Splice {
            detail: "the plot's items are not written as a block list of mappings".to_string(),
        })
}

/// The splice that writes `key: spelled` into the mapping at `route`: the
/// value replaced when the mapping carries the key, and one line added after
/// its last entry when it does not.
fn set_key(text: &str, route: Vec<PathPart>, key: &str, spelled: String) -> SpecEdit {
    match current_value(text, &route, key) {
        Some(current) => {
            let mut path = route;
            path.push(PathPart::from(key));
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
            key: key.to_string(),
            value: spelled,
        },
    }
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
