//! Putting a column on a chart's x, y or colour: the shelf's edit, as the list
//! of [`ChartEdit`]s that make it.
//!
//! **One gesture on the shelf can take more than one line of the spec.** The
//! generated map draws one picture in two dot layers, the whole table in
//! ghost ink and the selected subset over it, and each layer carries its own
//! `x:` and `y:` (see [`crate::chart_kinds::point_map_tile_sized`]). A column
//! put on the map's x has to move both, or the highlighted points leave the
//! grey cloud behind. So [`put_column`] moves the channel on every mark of the
//! plot that binds it, and on the first mark when none does:
//! `a_column_put_on_the_maps_x_moves_both_layers_and_takes_the_projection_out`
//! holds it on the map's two layers.
//!
//! **And the map is drawn as a map while its x and y hold the table's
//! coordinate pair**, the longitude and latitude the generator in
//! [`crate::dashboard`] drew it from, and as a dot plot when they do not. A
//! column's type does not decide it: `median_income` is quantitative,
//! as `longitude` is. A column put on either axis that takes the plot off the
//! pair takes `projectionType` out with it, so the page draws a dot plot with
//! axes to read that column by; putting the pair back puts the projection
//! back. A plot whose mapping does not cross that line keeps whatever
//! projection it has, because a projection the edit did not cause is the
//! analyst's.
//!
//! **Colour goes on the highlighted layer only.** The map's ghost layer is the
//! whole table in one ink, drawn so the selection has a cloud to cover, and a
//! `fill:` column on it would paint that cloud instead of the points the
//! analyst selected. So [`put_colour`] binds `fill` on the marks that read
//! through a selection (`filterBy:`), and on the first mark when no mark reads
//! through one, which `a_plot_with_no_selected_layer_takes_the_colour_on_its_first_mark`
//! holds. The page's legend is drawn from the plot's fill scale, and this
//! module does not place it. The dot renderer builds a fill scale a legend can be drawn from
//! for a column of strings and does not for a column of numbers, so a number
//! column put on colour reaches the page as a linear fill scale over the
//! column's range, in the mark ink and with no legend.
//! `a_column_put_on_the_maps_colour_paints_the_highlighted_layer_and_keeps_the_ghost_ink`
//! holds it on the map's two layers.
//!
//! **The edit comes back as the edits applied, in order**, because Save writes
//! the edits since the last Save into the chart file's text one at a time.
//! They are applied through [`edit::apply_for_fresh_load`] and not
//! [`edit::apply`]: the caller loads the page again from the edited spec, so
//! the axis titles a projected plot does not draw may appear, which is the
//! reload-from-disk gate's refusal and not this path's.

use std::fmt;

use brightfield_engine::ColumnProfile;
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{
    Component, Mark, MarkData, PlotNode, Spec, SpecValue, ValueOrParamRef,
};
use brightfield_spec::edit::{self, plot_at_path, ChartEdit, RefuseReason};
use brightfield_spec::layout::PlotAxis;

use crate::chart_kinds::POINT_MAP_PROJECTION;
use crate::dashboard::coordinate_pair;

/// The plot attribute a map is drawn through, as Mosaic spells it.
const PROJECTION_KEY: &str = "projectionType";

/// The channel a colour column is bound through, as Mosaic spells it.
const COLOUR_KEY: &str = "fill";

/// Why a column could not be put on a channel. The spec is left as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShelfRefusal {
    /// The table has no column by this name.
    NoSuchColumn(String),
    /// The edit layer found no target for one of the edits: no plot at the
    /// path, or no mark on it.
    Edit(RefuseReason),
}

impl fmt::Display for ShelfRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShelfRefusal::NoSuchColumn(name) => write!(f, "the table has no column named {name}"),
            ShelfRefusal::Edit(reason) => f.write_str(reason.clone().reason()),
        }
    }
}

impl std::error::Error for ShelfRefusal {}

/// **Put `column` on the `axis` channel of the plot at `plot`**, editing `spec`
/// in place, and return the [`ChartEdit`]s applied, in the order they were
/// applied.
///
/// `table` is the profile of the table the chart reads: `column` must be one
/// of its columns, and its coordinate pair, as the crate-private
/// `dashboard::coordinate_pair` finds it, decides whether the plot is a map
/// afterwards.
///
/// The list holds, in order:
///
/// 1. a [`ChartEdit::SetChannel`] for each mark on the plot that binds the
///    channel to a value other than `column`, in the plot's mark order, or
///    one for the first mark when no mark binds it;
/// 2. a [`ChartEdit::RemovePlotAttribute`] of `projectionType` when the plot
///    held the coordinate pair before and does not after, or a
///    [`ChartEdit::SetPlotAttribute`] of it when the plot comes to hold the
///    pair and carries no projection.
///
/// A column that is already where it is put yields no edits and leaves the
/// spec equal.
///
/// # Errors
///
/// [`ShelfRefusal::NoSuchColumn`] when the table has no `column`, and
/// [`ShelfRefusal::Edit`] when the plot path names no plot or the plot has no
/// mark. Either way `spec` is left as it was.
pub fn put_column(
    spec: &mut Spec,
    plot: &ComponentPath,
    axis: PlotAxis,
    column: &str,
    table: &[ColumnProfile],
) -> Result<Vec<ChartEdit>, ShelfRefusal> {
    if !table.iter().any(|c| c.name == column) {
        return Err(ShelfRefusal::NoSuchColumn(column.to_string()));
    }
    let channel = channel_key(axis);
    let target =
        plot_at_path(spec, &plot.0).ok_or(ShelfRefusal::Edit(RefuseReason::PlotNotFound))?;
    let pair = coordinate_pair(table)
        .map(|(lon, lat, _)| (table[lon].name.as_str(), table[lat].name.as_str()));
    let was_map = pair.is_some_and(|(lon, lat)| holds_pair(target, lon, lat));

    let marks = marks_of(target);
    if marks.is_empty() {
        return Err(ShelfRefusal::Edit(RefuseReason::NoSuchMark));
    }
    let binding: Vec<usize> = (0..marks.len())
        .filter(|&i| marks[i].options.contains_key(channel))
        .collect();
    let moving = if binding.is_empty() { vec![0] } else { binding };

    let mut edits: Vec<ChartEdit> = moving
        .into_iter()
        .filter(|&i| column_of(marks[i], channel) != Some(column))
        .map(|mark_ordinal| ChartEdit::SetChannel {
            plot: plot.clone(),
            mark_ordinal,
            channel: channel.to_string(),
            column: column.to_string(),
        })
        .collect();

    // The edits go onto a copy first, so a refusal part-way leaves the spec
    // as it was.
    let mut edited = spec.clone();
    for e in &edits {
        edit::apply_for_fresh_load(&mut edited, e).map_err(ShelfRefusal::Edit)?;
    }
    if let Some((lon, lat)) = pair {
        let after =
            plot_at_path(&edited, &plot.0).ok_or(ShelfRefusal::Edit(RefuseReason::PlotNotFound))?;
        let is_map = holds_pair(after, lon, lat);
        let projected = after.attributes.contains_key(PROJECTION_KEY);
        let projection = match (was_map, is_map) {
            (true, false) if projected => Some(ChartEdit::RemovePlotAttribute {
                plot: plot.clone(),
                key: PROJECTION_KEY.to_string(),
            }),
            (false, true) if !projected => Some(ChartEdit::SetPlotAttribute {
                plot: plot.clone(),
                key: PROJECTION_KEY.to_string(),
                value: SpecValue::String(POINT_MAP_PROJECTION.to_string()),
            }),
            _ => None,
        };
        if let Some(e) = projection {
            edit::apply_for_fresh_load(&mut edited, &e).map_err(ShelfRefusal::Edit)?;
            edits.push(e);
        }
    }

    *spec = edited;
    Ok(edits)
}

/// **Put `column` on the colour of the plot at `plot`**, editing `spec` in
/// place, and return the [`ChartEdit`]s applied, in the order they were
/// applied.
///
/// The list holds a [`ChartEdit::SetChannel`] of `fill` for each mark that
/// reads through a selection (`filterBy:`) and does not already bind `fill` to
/// `column`, in the plot's mark order, or one for the first mark when no mark
/// reads through a selection. A column put on a colour that already holds
/// another replaces it, and one already where it is put yields no edits and
/// leaves the spec equal. No edit touches a mark that does not read through a
/// selection when one does, so the map's ghost layer keeps its ink.
///
/// # Errors
///
/// [`ShelfRefusal::NoSuchColumn`] when the table has no `column`, and
/// [`ShelfRefusal::Edit`] when the plot path names no plot or the plot has no
/// mark. Either way `spec` is left as it was.
pub fn put_colour(
    spec: &mut Spec,
    plot: &ComponentPath,
    column: &str,
    table: &[ColumnProfile],
) -> Result<Vec<ChartEdit>, ShelfRefusal> {
    if !table.iter().any(|c| c.name == column) {
        return Err(ShelfRefusal::NoSuchColumn(column.to_string()));
    }
    let target =
        plot_at_path(spec, &plot.0).ok_or(ShelfRefusal::Edit(RefuseReason::PlotNotFound))?;
    let marks = marks_of(target);
    if marks.is_empty() {
        return Err(ShelfRefusal::Edit(RefuseReason::NoSuchMark));
    }
    let highlighted: Vec<usize> = (0..marks.len())
        .filter(|&i| reads_selection(marks[i]))
        .collect();
    let painted = if highlighted.is_empty() {
        vec![0]
    } else {
        highlighted
    };

    let edits: Vec<ChartEdit> = painted
        .into_iter()
        .filter(|&i| column_of(marks[i], COLOUR_KEY) != Some(column))
        .map(|mark_ordinal| ChartEdit::SetChannel {
            plot: plot.clone(),
            mark_ordinal,
            channel: COLOUR_KEY.to_string(),
            column: column.to_string(),
        })
        .collect();

    // The edits go onto a copy first, so a refusal part-way leaves the spec
    // as it was.
    let mut edited = spec.clone();
    for e in &edits {
        edit::apply_for_fresh_load(&mut edited, e).map_err(ShelfRefusal::Edit)?;
    }
    *spec = edited;
    Ok(edits)
}

/// The channel key an axis is bound through.
fn channel_key(axis: PlotAxis) -> &'static str {
    match axis {
        PlotAxis::X => "x",
        PlotAxis::Y => "y",
    }
}

/// The plot's marks, in order.
fn marks_of(plot: &PlotNode) -> Vec<&Mark> {
    plot.items
        .iter()
        .filter_map(|c| match c {
            Component::Mark(m) => Some(m),
            _ => None,
        })
        .collect()
}

/// Whether the mark's data reads through a selection: the highlighted layer of
/// a ghost-and-subset plot, whose `data:` carries a `filterBy:`.
fn reads_selection(mark: &Mark) -> bool {
    matches!(
        mark.data,
        Some(MarkData::From {
            filter_by: Some(_),
            ..
        })
    )
}

/// The column a mark binds `channel` to, when it binds it to a plain column
/// name rather than to an aggregate, a transform or a param.
fn column_of<'a>(mark: &'a Mark, channel: &str) -> Option<&'a str> {
    match mark.options.get(channel) {
        Some(ValueOrParamRef::Value(SpecValue::String(name))) => Some(name.as_str()),
        _ => None,
    }
}

/// Whether the plot draws the coordinate pair: at least one of its marks binds
/// x or y, and each mark that binds either binds x to `lon` and y to `lat`.
/// `putting_longitude_back_on_x_gives_the_spec_the_generator_wrote` reads it
/// turning true, and the two tests that take the projection out read it
/// turning false.
fn holds_pair(plot: &PlotNode, lon: &str, lat: &str) -> bool {
    let positional: Vec<&Mark> = plot
        .items
        .iter()
        .filter_map(|c| match c {
            Component::Mark(m) if m.options.contains_key("x") || m.options.contains_key("y") => {
                Some(m)
            }
            _ => None,
        })
        .collect();
    !positional.is_empty()
        && positional
            .iter()
            .all(|m| column_of(m, "x") == Some(lon) && column_of(m, "y") == Some(lat))
}
