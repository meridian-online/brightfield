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
//! **What is written.** The [`ChartEdit`] kinds listed here, each as a change
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
//! - A [`ChartEdit::RemoveColourLegend`], the edit that takes the item the one
//!   above wrote back out. Each colour legend item of the plot's list is taken
//!   out with arcform's delete of a list item, which removes the item's own
//!   lines, a comment flush above it by arcform's rule of comment ownership,
//!   and the blank lines that followed it; the lines around it stay as they
//!   were. A plot that holds none is the spec as it was, so the text comes
//!   back unchanged. Written back with [`ChartEdit::AddColourLegend`] the item
//!   lands after the list's last item, so it returns to its place when it was
//!   the last.
//! - A [`ChartEdit::PlaceColourLegend`], the edit that moves a plot's colour
//!   legend between its right, a band under it, and nowhere. Below is the
//!   plot's keys nested under a `vconcat:` at their place by arcform's nest,
//!   the plot given a `name:` when it had none, and a `legend: color` whose
//!   `for:` names it appended to the `vconcat`; out from below is the legend
//!   deleted and, when that leaves the plot alone in the `vconcat`, the plot's
//!   lines lifted back into its place by arcform's lift. The nest and the lift
//!   move each line with its comments, so a comment above a mark or at the end
//!   of its line stays with it
//!   (`each_legend_move_on_a_plot_in_a_concat_keeps_the_files_comments`). The
//!   legend each move writes is the one the reducer made, in the whole-spec
//!   serialiser's spelling; a comment on the legend's own lines is not kept.
//!   Between right and none the move is the item edits' splice.
//! - A [`ChartEdit::RemovePlotAttribute`], the edit that takes a map's
//!   projection out. The key's line is taken out. By arcform's rule of comment
//!   ownership a comment flush above the line, indented no deeper than it, is
//!   that line's header and goes with it; every other comment stays
//!   (`a_comment_flush_above_a_removed_line_goes_with_it_and_the_rest_stay`).
//!
//! Change mark type, add mark and remove mark are refused by kind until each
//! has a writer of its own.
//!
//! The splices are arcform's, so a placement move nests and lifts lines rather
//! than writing them: `SpecEdit::Nest` and `SpecEdit::Lift`, at the arcform
//! revision the workspace pins.
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
    apply_for_fresh_load, colour_legend_item_indices, colour_legends_below, mark_item_index,
    plot_at_path, plot_path_after, plot_route, ChartEdit, LegendPlacement, RefuseReason,
};
use brightfield_spec::layout::collect_legend_nodes;
use brightfield_spec::{
    parse_spec, serialise_spec, serialise_value, Component, Format, Spec, SpecValue,
};

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
    /// The value has no one-line spelling, and the edit is written as one
    /// line: an array that holds a mapping or a nested array, or a value YAML
    /// spells over several lines.
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
        ChartEdit::AddColourLegend { plot }
        | ChartEdit::RemoveColourLegend { plot }
        | ChartEdit::PlaceColourLegend { plot, .. } => (plot, LEGEND_KEY),
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

    // The splices to try, in order, each a batch applied in sequence; the
    // first whose text reads back as the edited spec is the one written.
    let splices = match edit {
        ChartEdit::SetPlotAttribute { key, value, .. } => {
            let spelled = one_line(key, value)?;
            vec![vec![set_key(text, route, key, spelled)]]
        }
        ChartEdit::RemovePlotAttribute { key, .. } => {
            if current_value(text, &route, key).is_none() {
                return Err(ChartTextRefusal::Inherited { key: key.clone() });
            }
            let mut path = route;
            path.push(PathPart::from(key.as_str()));
            vec![vec![SpecEdit::Delete { path }]]
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
                splices.push(vec![SpecEdit::RewriteFragment {
                    path,
                    from,
                    to: column.clone(),
                }]);
            }
            splices.push(vec![set_key(text, mark, channel, spelled)]);
            splices
        }
        ChartEdit::AddColourLegend { .. } => {
            vec![vec![append_legend_item(
                text,
                &parsed,
                &plot.0,
                route,
                COLOUR_LEGEND_ITEM,
            )?]]
        }
        ChartEdit::RemoveColourLegend { .. } => {
            vec![delete_legend_items(&parsed, &plot.0, route)]
        }
        ChartEdit::PlaceColourLegend { at, .. } => {
            let written = place_colour_legend(text, &parsed, &edited, edit, route, *at)?;
            let reads_back = parse_spec(&written, Format::Yaml).ok().map(|out| out.spec);
            return if reads_back.as_ref() == Some(&edited) {
                Ok(written)
            } else {
                Err(ChartTextRefusal::ReadsBackDifferently {
                    key: key.to_string(),
                })
            };
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
        match apply_yaml_edits(text, &splice) {
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

/// The splice that appends `item`, the text after an item's `- `, to the
/// `plot:` list of the plot at `route`, which is `plot_path` in `parsed`,
/// indented as the list's own items are. Its later lines, if it has any, are
/// indented two columns past the `- `.
fn append_legend_item(
    text: &str,
    parsed: &Spec,
    plot_path: &str,
    route: Vec<PathPart>,
    item: &str,
) -> Result<SpecEdit, ChartTextRefusal> {
    let last = plot_at_path(parsed, plot_path)
        .and_then(|p| p.items.len().checked_sub(1))
        .ok_or_else(|| ChartTextRefusal::Splice {
            detail: "the plot's list has no item to place a legend after".to_string(),
        })?;
    let mut list = route;
    list.push(PathPart::from("plot"));
    let indent = item_indent(text, &list, last)?;
    Ok(SpecEdit::Append {
        path: list,
        item: dashed(&indent, item),
    })
}

/// `item`, an item's text after its `- `, written as an item of a block list
/// whose dashes sit at `indent`.
fn dashed(indent: &str, item: &str) -> String {
    item.lines()
        .enumerate()
        .map(|(i, line)| match i {
            0 => format!("{indent}- {line}"),
            _ => format!("{indent}  {line}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The splices that take each colour legend item out of the `plot:` list of
/// the plot at `route`, which is `plot_path` in `parsed`.
fn delete_legend_items(parsed: &Spec, plot_path: &str, route: Vec<PathPart>) -> Vec<SpecEdit> {
    let items = plot_at_path(parsed, plot_path)
        .map(colour_legend_item_indices)
        .unwrap_or_default();
    let mut list = route;
    list.push(PathPart::from("plot"));
    // Last item first: each edit in a batch sees the text the one before it
    // left, so an item above the one being taken out has not moved when its
    // turn comes.
    items
        .into_iter()
        .rev()
        .map(|index| {
            let mut path = list.clone();
            path.push(PathPart::from(index));
            SpecEdit::Delete { path }
        })
        .collect()
}

/// The keys a chart file's root mapping holds for the spec rather than for its
/// root component — the keys the parser's root walk takes before it reads the
/// rest as the component (`parse.rs`, `walk_root`'s match).
const SPEC_KEYS: &[&str] = &["meta", "data", "params", "config", "plotDefaults"];

/// The key a below placement nests the plot's lines under.
const VCONCAT_KEY: &str = "vconcat";

/// [`ChartEdit::PlaceColourLegend`] written into `text`, which parses to
/// `parsed`; `edited` is the spec the reducer made of `parsed`, and `route` is
/// the plot's mapping.
///
/// The legend a move writes is the one in `edited`, spelled as the whole-spec
/// serialiser spells it, so each option the legend carries arrives as the
/// reducer carried it. A move between right and none is the item edit's
/// splice.
///
/// - To below, each colour legend item of the plot's list is deleted, the
///   plot is given its `name:` when it had none, its keys are nested under a
///   `vconcat:` at their place with arcform's nest, and the legend is appended
///   to the `vconcat` after the plot. The nest keeps every moved line's
///   comment.
/// - From below, each colour legend under the plot is deleted from the
///   `vconcat`, and when that leaves the plot alone in it, arcform's lift puts
///   the plot's lines in the `vconcat`'s place. The lift refuses a sequence of
///   two items, which is why the deletes come first. To right, the item is then
///   appended to the plot's list; to none, any colour legend item the plot
///   still holds is deleted.
fn place_colour_legend(
    text: &str,
    parsed: &Spec,
    edited: &Spec,
    edit: &ChartEdit,
    route: Vec<PathPart>,
    at: LegendPlacement,
) -> Result<String, ChartTextRefusal> {
    let plot_path = edit.plot_path();
    let splice = |text: &str, edits: &[SpecEdit]| {
        apply_yaml_edits(text, edits).map_err(|e| ChartTextRefusal::Splice {
            detail: e.to_string(),
        })
    };
    match (colour_legends_below(parsed, plot_path), at) {
        (Some(_), LegendPlacement::Below) => Ok(text.to_string()),
        (None, LegendPlacement::Right) => splice(
            text,
            &[append_legend_item(
                text,
                parsed,
                plot_path,
                route,
                COLOUR_LEGEND_ITEM,
            )?],
        ),
        (None, LegendPlacement::None) => {
            splice(text, &delete_legend_items(parsed, plot_path, route))
        }
        (None, LegendPlacement::Below) => {
            let mut edits = delete_legend_items(parsed, plot_path, route.clone());
            let wrapped = format!("{plot_path}/vconcat[0]");
            let name_of = |spec: &Spec, path: &str| {
                plot_at_path(spec, path).and_then(|p| p.attributes.get("name").cloned())
            };
            let name = name_of(edited, &wrapped);
            let mut keys = mapping_keys(text, &route);
            if name != name_of(parsed, plot_path) {
                let spelled = one_line("name", name.as_ref().unwrap_or(&SpecValue::Null))?;
                edits.push(set_key(text, route.clone(), "name", spelled));
                if !keys.iter().any(|k| k == "name") {
                    keys.push("name".to_string());
                }
            }
            edits.push(SpecEdit::Nest {
                path: route.clone(),
                keys,
                under: VCONCAT_KEY.to_string(),
            });
            let nested = splice(text, &edits)?;
            let legend = collect_legend_nodes(edited)
                .into_iter()
                .find(|(path, _)| *path == format!("{plot_path}/vconcat[1]"))
                .map(|(_, legend)| Component::Legend(legend.clone()))
                .ok_or_else(|| ChartTextRefusal::Splice {
                    detail: "the edit made no legend under the plot".to_string(),
                })?;
            let mut concat = route;
            concat.push(PathPart::from(VCONCAT_KEY));
            let indent = item_indent(&nested, &concat, 0)?;
            splice(
                &nested,
                &[SpecEdit::Append {
                    path: concat,
                    item: dashed(&indent, &component_text(legend)?),
                }],
            )
        }
        (Some(legends), LegendPlacement::Right | LegendPlacement::None) => {
            let parent = route[..route.len().saturating_sub(2)].to_vec();
            let mut concat = parent.clone();
            concat.push(PathPart::from(VCONCAT_KEY));
            let mut edits: Vec<SpecEdit> = legends
                .iter()
                .rev()
                .map(|&index| {
                    let mut path = concat.clone();
                    path.push(PathPart::from(index));
                    SpecEdit::Delete { path }
                })
                .collect();
            let after = plot_path_after(parsed, edit);
            let route = if after == plot_path {
                route
            } else {
                edits.push(SpecEdit::Lift {
                    path: parent.clone(),
                    key: VCONCAT_KEY.to_string(),
                });
                parent
            };
            let moved = splice(text, &edits)?;
            if at == LegendPlacement::None {
                return splice(&moved, &delete_legend_items(parsed, plot_path, route));
            }
            let item = plot_at_path(edited, &after)
                .and_then(|p| p.items.last())
                .filter(|_| plot_at_path(parsed, plot_path).is_some_and(|p| colour_legend_item_indices(p).is_empty()));
            match item {
                Some(item) => {
                    let item = component_text(item.clone())?;
                    splice(
                        &moved,
                        &[append_legend_item(&moved, parsed, plot_path, route, &item)?],
                    )
                }
                None => Ok(moved),
            }
        }
    }
}

/// The keys the mapping at `route` holds in `text`, in their order, but for the
/// spec's own keys when `route` is the document's root: the lines of the
/// component the mapping is.
fn mapping_keys(text: &str, route: &[PathPart]) -> Vec<String> {
    let Ok(mut node) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return Vec::new();
    };
    for part in route {
        node = match part {
            PathPart::Key(k) => node.get(k.as_str()).cloned().unwrap_or_default(),
            PathPart::Index(i) => node.get(*i).cloned().unwrap_or_default(),
        };
    }
    node.as_mapping()
        .map(|m| {
            m.keys()
                .filter_map(serde_yaml::Value::as_str)
                .filter(|k| !route.is_empty() || !SPEC_KEYS.contains(k))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `component` spelled as the whole-spec serialiser spells a root component:
/// its discriminator's line, then each option's, with no trailing newline.
fn component_text(component: Component) -> Result<String, ChartTextRefusal> {
    let spec = Spec {
        root: Some(component),
        ..Spec::default()
    };
    serialise_spec(&spec)
        .map(|text| text.trim_end().to_string())
        .map_err(|detail| ChartTextRefusal::Splice { detail })
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

/// `value` spelled as YAML on one line.
///
/// A scalar is spelled the way the whole-spec serialiser spells it. An array of
/// scalars is spelled in flow style, `[0, 100]`, with each string
/// double-quoted, `["#005389", "#f4f3f2"]`: the serialiser's block style spans
/// a line to an item, which is what a line-for-line splice cannot place. An
/// array that holds a mapping, a nested array or any other non-scalar value is
/// refused, because its flow spelling is not one the reader checks.
fn one_line(key: &str, value: &SpecValue) -> Result<String, ChartTextRefusal> {
    let not_one_line = || ChartTextRefusal::ValueNotOneLine {
        key: key.to_string(),
    };
    if let SpecValue::Array(items) = value {
        let spelled = items
            .iter()
            .map(|item| flow_scalar(item).ok_or_else(not_one_line))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(format!("[{}]", spelled.join(", ")));
    }
    let spelled = serialise_value(value).map_err(|_| not_one_line())?;
    match spelled.strip_suffix('\n') {
        Some(line) if !line.contains('\n') => Ok(line.to_string()),
        _ => Err(not_one_line()),
    }
}

/// One item of a flow-style array, or `None` when the item is not a scalar.
///
/// A string is written double-quoted with JSON's escapes, which YAML reads the
/// same way, so a hex colour keeps the `#` that would otherwise start a
/// comment. The other scalars are spelled as the whole-spec serialiser spells
/// them.
fn flow_scalar(item: &SpecValue) -> Option<String> {
    match item {
        SpecValue::String(s) => serde_json::to_string(s).ok(),
        SpecValue::Null | SpecValue::Bool(_) | SpecValue::Integer(_) | SpecValue::Float(_) => {
            let spelled = serialise_value(item).ok()?;
            let line = spelled.strip_suffix('\n')?;
            (!line.contains('\n')).then(|| line.to_string())
        }
        _ => None,
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
