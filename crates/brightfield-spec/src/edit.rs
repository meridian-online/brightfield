//! The ChartEdit spine — typed structural mutations of the working chart Spec.
//!
//! Named `ChartEdit` to keep it distinct from arc's `arc::spec::SpecEdit` (the
//! manifest splice op): this is a mutation of the working *chart* AST, not of a
//! protocol manifest. The keyboard grammar named five reserved verbs (`m` /
//! `a` / `e` / `d` / undo) — a way to change a mark's type, add a mark, bind a
//! channel, remove a mark, and undo — applied live, then committed on a
//! deliberate action. This module is the substrate behind them:
//! a framework-free [`ChartEdit`] enum + [`apply`] reducer that walks the root
//! [`Component`] tree by focused-plot path and mutates the AST in place, a
//! snapshot [`UndoStack`] with a commit barrier, and a [`classify_edit`]
//! gate-classifier that REIMPLEMENTS the app-binary reload gate
//! (`same_layout` / `chrome_divergence`) from the spec representation, so a
//! within-plot edit that WOULD bounce to "restart to apply" is refused at edit
//! time with a reason instead.
//!
//! No UI-framework type crosses this boundary — the shell layer is the shim that drives
//! the reducer and re-renders (the standing framework-free rule, mirroring
//! `brightfield-keys` / `spec_save`). Targeting is by focused-plot + ordinal
//! (v1: the plot's PRIMARY/first mark): count-changing edits re-walk the live
//! AST every time and never cache a positional path string, so a within-plot
//! insert/remove that renumbers later siblings can't corrupt a stored path.

use indexmap::IndexMap;

use crate::analysis::ComponentPath;
use crate::ast::{
    Component, ConcatNode, LegendNode, Mark, PlotNode, Spec, SpecValue, ValueOrParamRef,
};
use crate::layout::{
    below_legends, collect_legend_nodes, collect_plot_nodes, resolve_axis_titles, AxisTitle, Rect,
};
use crate::vocab::{LegendChannel, MarkKind};

/// Positional channel keys inherited by an added mark from the plot's primary
/// mark so the new mark actually renders against the same frame (data source +
/// x/y). Only positional channels are inherited (a colour channel would render
/// differently and is a deliberate author choice, not an inheritance).
const INHERITED_CHANNELS: &[&str] = &["x", "y", "x1", "x2", "y1", "y2"];

/// A typed structural mutation applied to the working [`Spec`] by [`apply`] —
/// the framework-free AST-mutation API the keyboard grammar named as missing.
///
/// The reserved undo verb is an [`UndoStack`] pop, not an edit. An edit is
/// TYPED (not an exec-string, per the VisiData warning), walks the live AST
/// via a plot [`ComponentPath`], and is bracketed by a whole-`Spec` clone
/// snapshot so undo is total and near-free. [`ChartEdit::ChangeMarkType`],
/// [`ChartEdit::AddMark`], [`ChartEdit::SetChannel`] and
/// [`ChartEdit::RemoveMark`] target the focused plot's primary mark;
/// [`ChartEdit::SetPlotAttribute`] and [`ChartEdit::RemovePlotAttribute`]
/// target the plot's own attribute map instead, and
/// [`ChartEdit::AddColourLegend`] and [`ChartEdit::RemoveColourLegend`] put a
/// legend into the plot's list of items and take it out, and
/// [`ChartEdit::PlaceColourLegend`] moves it between the plot's items, a
/// `vconcat` under the plot, and out of the file. [`ChartEdit::AddMark`]
/// and [`ChartEdit::RemoveMark`] are count-CHANGING; [`ChartEdit::ChangeMarkType`],
/// [`ChartEdit::SetChannel`], [`ChartEdit::SetPlotAttribute`],
/// [`ChartEdit::RemovePlotAttribute`], [`ChartEdit::AddColourLegend`],
/// [`ChartEdit::RemoveColourLegend`] and [`ChartEdit::PlaceColourLegend`] are
/// count-STABLE. The transient apply
/// treats the two groups differently (the coordinator flat-index rebuild). The
/// count is the plot's marks: a legend is not one.
#[derive(Debug, Clone, PartialEq)]
pub enum ChartEdit {
    /// Retype the focused plot's primary mark (`dot` -> `bar`). Count-stable.
    /// Among the SimpleLowerer family the SQL is byte-identical — the real
    /// change is the renderer/scene geometry.
    ChangeMarkType {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
        /// Ordinal of the target mark among the plot's marks (v1: always 0).
        mark_ordinal: usize,
        /// The new mark kind.
        new_kind: MarkKind,
    },
    /// Append a new mark of `kind` to the focused plot's items (order-
    /// preserving). Count-CHANGING. The new mark inherits the primary mark's
    /// data source + positional channels so it renders against the same frame.
    AddMark {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
        /// The kind of the mark to add (the argument-overlay payload).
        kind: MarkKind,
    },
    /// Bind `channel` (`x`/`y`/...) to `column` on the primary mark. Count-
    /// stable. Changes the SELECT.
    SetChannel {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
        /// Ordinal of the target mark among the plot's marks (v1: always 0).
        mark_ordinal: usize,
        /// The channel key (wire name, e.g. `x`).
        channel: String,
        /// The column to bind.
        column: String,
    },
    /// Drop the primary mark from the focused plot. Count-CHANGING. Refused if
    /// it would empty the plot.
    RemoveMark {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
        /// Ordinal of the target mark among the plot's marks (v1: always 0).
        mark_ordinal: usize,
    },
    /// Write `key: value` into the focused plot's own attribute map — the
    /// siblings of `plot:`, not a mark's channels. Count-stable, and the one
    /// variant that targets no mark at all.
    ///
    /// The plot attribute a surface changes today is the positional scale
    /// type (`xScale: log`), which re-bins and re-draws every mark on the plot
    /// through one key. The variant is written in terms of the attribute map
    /// rather than in terms of scales because that map is what the AST holds,
    /// and because [`classify_edit`] already refuses the attributes that would
    /// move launch-fixed chrome: an `xLabel` written here changes the axis
    /// title and comes back [`RefuseReason::WouldChangeAxisTitle`], with the
    /// spec untouched. `an_x_label_written_as_a_plot_attribute_is_refused`
    /// holds that.
    SetPlotAttribute {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
        /// The attribute key, as the spec spells it (`xScale`).
        key: String,
        /// The value to write.
        value: SpecValue,
    },
    /// Take `key` out of the focused plot's own attribute map, so the plot
    /// draws that attribute's default. Count-stable, and like
    /// [`ChartEdit::SetPlotAttribute`] it targets no mark.
    ///
    /// Writing the default's value back is not the same edit: `yScale: linear`
    /// leaves a key that says the analyst chose a linear scale, where the spec
    /// they started from said nothing. The key is removed in place
    /// (order-preserving), so every other attribute keeps its position.
    ///
    /// A key the plot does not carry is a no-op that leaves the spec equal.
    /// The gate is the one [`ChartEdit::SetPlotAttribute`] has: dropping an
    /// `xLabel` turns an overridden or suppressed axis title back into a
    /// derived one, which comes back [`RefuseReason::WouldChangeAxisTitle`]
    /// with the spec untouched.
    RemovePlotAttribute {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
        /// The attribute key to remove, as the spec spells it (`yScale`).
        key: String,
    },
    /// Append `legend: color` to the focused plot's items, after the last of
    /// them. Count-stable, and targets no mark: a legend is a plot item and
    /// not a mark, which is why [`ChartEdit::AddMark`] cannot write it.
    ///
    /// Mosaic draws the item to the right of the plot's picture, so no concat
    /// is written for it. A plot whose items already hold a colour legend is
    /// left equal: the edit is idempotent, and a second one is not a second
    /// legend. A colour legend that sits outside the plot and names it with
    /// `for:` is not an item of the plot and is not looked for here; whether
    /// to write this edit over one is the caller's question
    /// ([`colour_legend_covers`]).
    AddColourLegend {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
    },
    /// Take the colour legend out of the focused plot's items, so the plot's
    /// picture takes the room it drew. Count-stable, and like
    /// [`ChartEdit::AddColourLegend`] it targets no mark: each mark and each
    /// other item stays where it was.
    ///
    /// Every item of the plot that is a colour legend goes, so the plot holds
    /// none after the edit. A plot that holds none is left equal. A legend for
    /// another channel is not a colour legend and stays, and a colour legend
    /// outside the plot that names it with `for:` is not an item of the plot
    /// and stays too: the edit takes out what [`ChartEdit::AddColourLegend`]
    /// puts in, and no more.
    RemoveColourLegend {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
    },
    /// Put the focused plot's colour legend at `at`: to the right of the
    /// plot's picture, under it, or nowhere. Count-stable, and targets no
    /// mark. Unlike every other edit it can change the tree the plot sits in,
    /// so a later edit finds the plot at [`plot_path_after`]'s path.
    ///
    /// Right is a `legend: color` item of the plot, the item
    /// [`ChartEdit::AddColourLegend`] writes. Below is the one shape a Mosaic
    /// file has for a legend under its plot: a `vconcat` whose entries are the
    /// plot, carrying a `name:`, and a `legend: color` whose `for:` is that
    /// name. A plot is below when [`crate::layout::below_legends`] reads it so,
    /// which is the rule the page draws the band by, so the edit and the page
    /// agree on what below is.
    ///
    /// To below, the plot's colour legend items come out, and at the plot's
    /// place a `vconcat` of two entries goes in: the plot, and the standalone
    /// legend carrying each option the first item carried. A plot with no
    /// `name:` is given one no plot of the file holds and no legend's `for:`
    /// names ([`fresh_plot_name`]); a plot with one keeps it.
    ///
    /// From below, each colour legend drawn under the plot is taken out of its
    /// `vconcat`, and when that leaves the plot the `vconcat`'s one entry, the
    /// plot takes the `vconcat`'s place. A `vconcat` that holds another entry
    /// keeps it and stays where it was. To right, the item then goes back
    /// among the plot's items, after the last of them, carrying the first
    /// standalone legend's options but its `for:`. The name the plot was given
    /// stays, so right to below and back is the spec as it was but for that
    /// name, when the item was the plot's last.
    ///
    /// A plot already at `at` is left equal.
    PlaceColourLegend {
        /// Plot-node path of the focused plot.
        plot: ComponentPath,
        /// Where the plot's colour legend goes.
        at: LegendPlacement,
    },
}

/// Where a plot's colour legend is drawn, as the legend row's three values
/// name it, and as [`ChartEdit::PlaceColourLegend`] writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendPlacement {
    /// A `legend: color` item of the plot, drawn to the right of its picture.
    Right,
    /// A `vconcat` of the named plot and a `legend: color` whose `for:` names
    /// it, drawn in a band under the plot.
    Below,
    /// No colour legend for the plot.
    None,
}

impl LegendPlacement {
    /// The value as the command log and the legend row spell it.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            LegendPlacement::Right => "right",
            LegendPlacement::Below => "below",
            LegendPlacement::None => "none",
        }
    }
}

impl ChartEdit {
    /// The plot-node path this edit targets.
    #[must_use]
    pub fn plot_path(&self) -> &str {
        match self {
            ChartEdit::ChangeMarkType { plot, .. }
            | ChartEdit::AddMark { plot, .. }
            | ChartEdit::SetChannel { plot, .. }
            | ChartEdit::RemoveMark { plot, .. }
            | ChartEdit::SetPlotAttribute { plot, .. }
            | ChartEdit::RemovePlotAttribute { plot, .. }
            | ChartEdit::AddColourLegend { plot }
            | ChartEdit::RemoveColourLegend { plot }
            | ChartEdit::PlaceColourLegend { plot, .. } => plot.0.as_str(),
        }
    }

    /// The edit's kind as the command log spells it (`change-mark-type`,
    /// `set-plot-attribute`), with no target and no value — the head of
    /// [`ChartEdit::summary`], and the name a refusal of the whole kind gives.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            ChartEdit::ChangeMarkType { .. } => "change-mark-type",
            ChartEdit::AddMark { .. } => "add-mark",
            ChartEdit::SetChannel { .. } => "set-channel",
            ChartEdit::RemoveMark { .. } => "remove-mark",
            ChartEdit::SetPlotAttribute { .. } => "set-plot-attribute",
            ChartEdit::RemovePlotAttribute { .. } => "remove-plot-attribute",
            ChartEdit::AddColourLegend { .. } => "add-colour-legend",
            ChartEdit::RemoveColourLegend { .. } => "remove-colour-legend",
            ChartEdit::PlaceColourLegend { .. } => "place-colour-legend",
        }
    }

    /// Whether this edit changes the mark COUNT (AddMark / RemoveMark) — the
    /// transient apply must rebuild the coordinator + engine flat-index maps
    /// for a count-changing edit.
    #[must_use]
    pub fn is_count_changing(&self) -> bool {
        matches!(
            self,
            ChartEdit::AddMark { .. } | ChartEdit::RemoveMark { .. }
        )
    }

    /// The mark ordinal this edit targets (v1: always 0, the primary mark);
    /// `AddMark` appends, so it reports 0. The count-stable in-place coordinator
    /// mutation indexes `mark_indices` by this so it matches the reducer's
    /// nth-mark mutation rather than assuming the first mark (finding 7).
    #[must_use]
    pub fn mark_ordinal(&self) -> usize {
        match self {
            ChartEdit::ChangeMarkType { mark_ordinal, .. }
            | ChartEdit::SetChannel { mark_ordinal, .. }
            | ChartEdit::RemoveMark { mark_ordinal, .. } => *mark_ordinal,
            ChartEdit::AddMark { .. }
            | ChartEdit::SetPlotAttribute { .. }
            | ChartEdit::RemovePlotAttribute { .. }
            | ChartEdit::AddColourLegend { .. }
            | ChartEdit::RemoveColourLegend { .. }
            | ChartEdit::PlaceColourLegend { .. } => 0,
        }
    }

    /// A short human-readable summary for the command-log panel
    /// (`change-mark-type: -> bar`).
    #[must_use]
    pub fn summary(&self) -> String {
        let kind = self.kind_name();
        match self {
            ChartEdit::ChangeMarkType { new_kind, .. } => {
                format!("{kind}: -> {}", new_kind.wire_name())
            }
            ChartEdit::AddMark { kind: mark, .. } => format!("{kind}: {}", mark.wire_name()),
            ChartEdit::SetChannel {
                channel, column, ..
            } => {
                format!("{kind}: {channel} -> {column}")
            }
            ChartEdit::RemoveMark { .. }
            | ChartEdit::AddColourLegend { .. }
            | ChartEdit::RemoveColourLegend { .. } => kind.to_string(),
            ChartEdit::SetPlotAttribute { key, value, .. } => match value {
                SpecValue::String(s) => format!("{kind}: {key} -> {s}"),
                other => format!("{kind}: {key} -> {other:?}"),
            },
            ChartEdit::RemovePlotAttribute { key, .. } => format!("{kind}: {key}"),
            ChartEdit::PlaceColourLegend { at, .. } => format!("{kind}: {}", at.wire_name()),
        }
    }
}

/// Why an edit was refused WITHOUT mutating the Spec — the reload gate would
/// otherwise bounce a committed version of it to "restart to apply", or the
/// edit's target does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefuseReason {
    /// The focused plot path did not resolve to a plot node.
    PlotNotFound,
    /// The plot has no mark at the requested ordinal (v1: no primary mark).
    NoSuchMark,
    /// Removing this mark would leave the plot empty — a within-plot edit must
    /// not empty a plot (trips `same_layout`); v1 refuses it.
    WouldEmptyPlot,
    /// Rebinding a DERIVED x/y axis would change the axis title (derived
    /// from the encoding's column name), which grows the
    /// launch-fixed margins — a `chrome_divergence` a reload can't hot-apply. v1
    /// refuses it; bind such an axis on a plot with an explicit `xLabel`/`yLabel`
    /// (an Override / Suppress axis is title-stable under a rebind).
    WouldChangeAxisTitle,
    /// Retyping to a mark of a DIFFERENT zero-baseline class (e.g. `dot` -> `bar`)
    /// would flip the axis-inset default on the value axis (a
    /// zero-baseline end stays flush), a launch-fixed `chrome_divergence`. v1
    /// allows a retype only WITHIN the same zero-baseline class (dot<->line,
    /// barY<->areaY<->rectY, ...).
    WouldChangeInset,
    /// Changing a colour scale (a `fill`/`stroke` rebind, or a retype that
    /// adds/removes a sequential-colour renderer) on a plot a STANDALONE colour
    /// `legend:` references would change that legend's swatches/gradient — a
    /// `chrome_divergence` a reload can't hot-apply (finding 3). An
    /// INLINE colour fill with no referencing legend stays clean (not captured by
    /// the gate); v1 refuses only the legend-referenced case.
    WouldChangeLegend,
    /// Moving a plot's colour legend under it, or out from under it, adds or
    /// takes out a standalone legend in a `vconcat` and changes the plot's
    /// rect — a `same_layout` divergence a reload can't hot-apply. A page
    /// loaded afresh lays the band out again ([`apply_for_fresh_load`]).
    WouldChangeLayout,
}

impl RefuseReason {
    /// A human-readable reason, surfaced in the command-log panel / a rejection.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            RefuseReason::PlotNotFound => "no focused plot to edit",
            RefuseReason::NoSuchMark => "the focused plot has no mark to edit",
            RefuseReason::WouldEmptyPlot => {
                "would empty the plot (removing a plot's last mark needs a restart)"
            }
            RefuseReason::WouldChangeAxisTitle => {
                "would change a derived axis title (label the axis with xLabel/yLabel first)"
            }
            RefuseReason::WouldChangeInset => {
                "would change the axis-inset baseline (retype within the same bar/area/dot class)"
            }
            RefuseReason::WouldChangeLegend => {
                "would change a colour legend's scale (remove the standalone legend, or edit its plot's colour, first)"
            }
            RefuseReason::WouldChangeLayout => {
                "would move a colour legend into or out of the band under its plot (the page has to be laid out again)"
            }
        }
    }
}

/// Apply a structural edit to the working Spec IN PLACE, or return
/// `Err(RefuseReason)` WITHOUT mutating when the edit would trip a reload gate
/// or its target does not exist.
///
/// The classifier runs first ([`classify_edit`]) so a gate-tripping edit never
/// mutates; the caller snapshots the pre-edit Spec BEFORE calling apply and, on
/// `Err`, discards the snapshot (nothing changed).
pub fn apply(spec: &mut Spec, edit: &ChartEdit) -> Result<(), RefuseReason> {
    // Gate FIRST — a refused edit must leave the Spec byte-identical.
    classify_edit(spec, edit)?;
    apply_unchecked(spec, edit);
    Ok(())
}

/// Apply a structural edit to the working Spec IN PLACE for a caller that then
/// **loads the page afresh from the edited spec**, or return
/// `Err(RefuseReason)` WITHOUT mutating when the edit has no target.
///
/// [`apply`]'s chrome refusals ([`RefuseReason::WouldChangeInset`],
/// [`RefuseReason::WouldChangeAxisTitle`], [`RefuseReason::WouldChangeLegend`])
/// exist for a reload from disk, which swaps new plot scenes into chrome that
/// was laid out when the window launched. A page loaded afresh lays its
/// margins, titles and legends out again from the spec it is given, so an
/// axis title that changes is drawn rather than bounced. The shell's tile
/// controls rebuild the page that way after every edit.
///
/// What stays refused is what no page could be built over: a plot path that
/// names no plot ([`RefuseReason::PlotNotFound`]), a mark ordinal past the
/// plot's marks ([`RefuseReason::NoSuchMark`]), and a removal of a plot's last
/// mark ([`RefuseReason::WouldEmptyPlot`]) — the checks [`classify_edit`]
/// makes before it compares any chrome, shared rather than restated.
///
/// The reload-from-disk path keeps calling [`apply`], and its refusals are
/// unchanged by this entry point existing.
pub fn apply_for_fresh_load(spec: &mut Spec, edit: &ChartEdit) -> Result<(), RefuseReason> {
    check_target(spec, edit)?;
    apply_unchecked(spec, edit);
    Ok(())
}

/// Mutate the spec IN PLACE for `edit`, ASSUMING [`classify_edit`]'s structural
/// preconditions already hold (the plot + target mark exist). Never called by
/// the app directly — [`apply`] gates first — but shared with the classifier,
/// which applies it to a CLONE to compute the post-edit chrome signature.
fn apply_unchecked(spec: &mut Spec, edit: &ChartEdit) {
    if let ChartEdit::PlaceColourLegend { plot, at } = edit {
        place_colour_legend(spec, &plot.0, *at);
        return;
    }
    let Some(p) = plot_at_path_mut(spec, edit.plot_path()) else {
        return;
    };
    match edit {
        ChartEdit::ChangeMarkType {
            mark_ordinal,
            new_kind,
            ..
        } => {
            if let Some(item) = nth_mark_item_index(p, *mark_ordinal) {
                if let Component::Mark(m) = &mut p.items[item] {
                    m.kind = *new_kind;
                    m.status = new_kind.status();
                }
            }
        }
        ChartEdit::AddMark { kind, .. } => {
            // Inherit the primary mark's data source + positional channels so
            // the added mark renders against the same frame.
            let (data, options) = p
                .items
                .iter()
                .find_map(|c| match c {
                    Component::Mark(m) => Some((m.data.clone(), inherited_positional(&m.options))),
                    _ => None,
                })
                .unwrap_or((None, IndexMap::new()));
            p.items.push(Component::Mark(Mark {
                kind: *kind,
                status: kind.status(),
                data,
                options,
            }));
        }
        ChartEdit::SetPlotAttribute { key, value, .. } => {
            p.attributes.insert(key.clone(), value.clone());
        }
        ChartEdit::RemovePlotAttribute { key, .. } => {
            // `shift_remove`, not `swap_remove`: the last attribute must not
            // take the removed one's place in the map's order.
            p.attributes.shift_remove(key);
        }
        ChartEdit::AddColourLegend { .. } => {
            if !holds_colour_legend(p) {
                p.items.push(Component::Legend(LegendNode {
                    channel: LegendChannel::Color,
                    status: LegendChannel::Color.status(),
                    options: IndexMap::new(),
                }));
            }
        }
        ChartEdit::RemoveColourLegend { .. } => {
            // `retain` keeps the survivors in their order, so each mark and
            // each other item stays where it was relative to the rest.
            p.items.retain(|c| !is_colour_legend(c));
        }
        ChartEdit::SetChannel {
            mark_ordinal,
            channel,
            column,
            ..
        } => {
            if let Some(item) = nth_mark_item_index(p, *mark_ordinal) {
                if let Component::Mark(m) = &mut p.items[item] {
                    m.options.insert(
                        channel.clone(),
                        ValueOrParamRef::Value(SpecValue::String(column.clone())),
                    );
                }
            }
        }
        ChartEdit::RemoveMark { mark_ordinal, .. } => {
            if let Some(item) = nth_mark_item_index(p, *mark_ordinal) {
                p.items.remove(item);
            }
        }
        // Placed above, before the plot is looked up: the edit reaches past
        // the plot into the tree it sits in.
        ChartEdit::PlaceColourLegend { .. } => {}
    }
}

/// Classify whether a pending edit would trip a reload gate or has no valid
/// target — WITHOUT mutating. This expresses the reload gate
/// (`same_layout` / `chrome_divergence`) from the spec representation. It was
/// born as a reimplementation of the retired gpui shell's gate and pinned
/// equal by an agreement test in that binary; with the gpui shell deleted,
/// this classifier is the single authority on gate verdicts.
///
/// Two structural preconditions refuse first (a missing target mark; a
/// `RemoveMark` that would EMPTY the plot). Then the WITHIN-PLOT chrome signature
/// is diffed BEFORE vs AFTER the edit (applied to a clone): the axis-inset
/// baseline SET (an axis end is flush iff ANY mark zero-baselines it)
/// and the DERIVED x/y axis titles (a Derive axis takes the first
/// mark's column). A difference is refused ([`RefuseReason::WouldChangeInset`] /
/// [`RefuseReason::WouldChangeAxisTitle`]) because both feed launch-fixed chrome
/// a reload can't hot-apply. Everything a within-plot mark edit CANNOT change is
/// gate-clean and needs no check: plot count/geometry (`same_layout`), the
/// colorScheme, the dashboard title, standalone legends, inline-legend
/// suppression — so binding an inline `fill` (NOT captured by the gate) is
/// allowed. The inset check is CONSERVATIVE on a categorical axis (the gate
/// applies no inset default there, so a baseline flip is inert): it may
/// over-refuse, which is the safe side (never a silent bounce).
pub fn classify_edit(spec: &Spec, edit: &ChartEdit) -> Result<(), RefuseReason> {
    let plot = check_target(spec, edit)?;

    // A move into or out of the band under the plot is a change of layout,
    // and it moves the plot, so the clone below would not find it at the
    // edit's path. Between right and none the plot stays where it is and the
    // comparison below holds, as it does for the item edits.
    if let ChartEdit::PlaceColourLegend { plot, at } = edit {
        if legends_below(spec, &plot.0).is_some() != (*at == LegendPlacement::Below) {
            return Err(RefuseReason::WouldChangeLayout);
        }
    }

    // Apply the edit to a clone ONCE — both the colour-legend gate and the
    // inset/title chrome comparison diff the launch-fixed chrome against it.
    let mut clone = spec.clone();
    apply_unchecked(&mut clone, edit);
    let after_plot = plot_at_path(&clone, edit.plot_path()).ok_or(RefuseReason::PlotNotFound)?;

    // Colour-scale change under a standalone colour legend (finding 3 + delta
    // finding 2). An explicit `legend: color for: <this plot>` renders the plot's
    // fill/stroke colour scale, which `chrome_divergence` captures — so a colour
    // rebind (or a retype that adds/removes a sequential-colour renderer) trips
    // the real gate. A no-`for:` colour legend is placed only when the dashboard
    // has EXACTLY ONE colour-encoded plot (`resolve_legends`), so a colour edit
    // that flips that count (0->1 shows the legend, 1->2 hides the sole one) — or
    // that changes the sole colour plot's own domain — trips it too. Either way,
    // refuse rather than silently bounce to "restart to apply". An inline colour
    // fill with NO standalone legend stays clean (the earlier finding — see
    // `binding_an_inline_fill_is_clean`).
    let colour_edit = match edit {
        ChartEdit::SetChannel { channel, .. } => is_colour_channel(channel),
        ChartEdit::ChangeMarkType {
            new_kind,
            mark_ordinal,
            ..
        } => {
            let current = nth_mark_kind(plot, *mark_ordinal);
            kind_carries_colour_scale(*new_kind) || current.is_some_and(kind_carries_colour_scale)
        }
        _ => false,
    };
    if colour_edit && colour_legend_chrome_changes(spec, &clone, plot, after_plot) {
        return Err(RefuseReason::WouldChangeLegend);
    }

    // Chrome-signature comparison: diff the launch-fixed chrome the reload gate
    // compares (inset baselines + derived axis titles).
    let before = plot_chrome_signature(plot);
    let after = plot_chrome_signature(after_plot);

    if before.baseline_x != after.baseline_x || before.baseline_y != after.baseline_y {
        return Err(RefuseReason::WouldChangeInset);
    }
    if before.x_title != after.x_title || before.y_title != after.y_title {
        return Err(RefuseReason::WouldChangeAxisTitle);
    }
    Ok(())
}

/// The structural preconditions every edit meets before anything is applied or
/// compared: the plot exists, the target mark exists, and a removal leaves the
/// plot a mark. Hands back the plot, which [`classify_edit`] goes on to diff.
fn check_target<'a>(spec: &'a Spec, edit: &ChartEdit) -> Result<&'a PlotNode, RefuseReason> {
    let plot = plot_at_path(spec, edit.plot_path()).ok_or(RefuseReason::PlotNotFound)?;
    let mark_count = plot
        .items
        .iter()
        .filter(|c| matches!(c, Component::Mark(_)))
        .count();
    match edit {
        ChartEdit::ChangeMarkType { mark_ordinal, .. }
        | ChartEdit::SetChannel { mark_ordinal, .. }
        | ChartEdit::RemoveMark { mark_ordinal, .. }
            if *mark_ordinal >= mark_count =>
        {
            Err(RefuseReason::NoSuchMark)
        }
        ChartEdit::RemoveMark { .. } if mark_count <= 1 => Err(RefuseReason::WouldEmptyPlot),
        _ => Ok(plot),
    }
}

/// The launch-fixed chrome a plot contributes to `chrome_divergence` that a
/// within-plot mark edit can perturb: the axis-inset baseline set + the resolved
/// x/y axis titles.
struct PlotChromeSig {
    baseline_x: bool,
    baseline_y: bool,
    x_title: Option<String>,
    y_title: Option<String>,
}

fn plot_chrome_signature(plot: &PlotNode) -> PlotChromeSig {
    let mut baseline_x = false;
    let mut baseline_y = false;
    for c in &plot.items {
        if let Component::Mark(m) = c {
            match mark_zero_baseline_axis(m.kind) {
                Some("x") => baseline_x = true,
                Some("y") => baseline_y = true,
                _ => {}
            }
        }
    }
    let decided = resolve_axis_titles(plot);
    PlotChromeSig {
        baseline_x,
        baseline_y,
        x_title: resolve_derived_title(&decided.x, plot, "x"),
        y_title: resolve_derived_title(&decided.y, plot, "y"),
    }
}

/// Whether a channel key drives a COLOUR scale (finding 3) — a rebind of one
/// changes the colour domain/scheme a standalone legend renders.
///
/// This does NOT distinguish `fill` from `stroke`: a `stroke` rebind under a
/// legend that displays only the `fill` scale is refused too (delta finding 4).
/// That is deliberate safe-side conservatism — the classifier's job is to never
/// LET a silent "restart to apply" bounce through, and over-refusing a rare
/// stroke-under-a-fill-legend edit is the cheap, correct-direction error (matches
/// the classifier's documented conservatism). Narrowing to the scale the legend
/// actually displays is a possible future refinement, not a correctness gap.
fn is_colour_channel(channel: &str) -> bool {
    matches!(channel, "fill" | "stroke" | "color" | "colour")
}

/// Whether a mark kind's RENDERER carries a sequential colour scale (finding 3)
/// — mirrors `configured_renderer`'s scheme-carrying set. A retype that adds or
/// removes one changes what a standalone colour legend would render.
fn kind_carries_colour_scale(kind: MarkKind) -> bool {
    matches!(
        kind,
        MarkKind::Raster
            | MarkKind::Heatmap
            | MarkKind::Cell
            | MarkKind::Hexbin
            | MarkKind::Contour
    )
}

/// The kind of the `ordinal`-th mark in a plot (finding 3 — the current colour
/// state a retype changes away from).
fn nth_mark_kind(plot: &PlotNode, ordinal: usize) -> Option<MarkKind> {
    let item = nth_mark_item_index(plot, ordinal)?;
    match &plot.items[item] {
        Component::Mark(m) => Some(m.kind),
        _ => None,
    }
}

/// Whether a colour edit on `focused` changes the chrome a STANDALONE colour
/// `legend:` renders — the precise reproduction of the real reload gate's
/// `legends` divergence (finding 3 + delta finding 2). Compares the pre-edit
/// spec/plot (`before` / `focused_before`) to the post-edit clone (`after` /
/// `focused_after`):
///
///   - An explicit `legend: color for: <name>` renders THAT plot's colour scale
///     regardless of the global count, so a colour edit matters only when it
///     targets the focused plot (the original finding 3).
///   - A no-`for:` colour legend is placed only when EXACTLY ONE colour-encoded
///     plot exists (`resolve_legends`). Its chrome changes when that placement
///     FLIPS (0->1 shows it, 1->2 hides the sole one), or when it stays placed
///     and the focused plot — the only plot a single edit touches — is the sole
///     colour plot whose domain it renders (delta finding 2: gating only on the
///     PRE-edit focused plot's colour-encoding missed the 0->1 / 1->2 flips).
///   - A `$param for:` can't be resolved statically — the reload gate backstops.
///
/// Inline legends (drawn inside a plot) are NOT standalone and are not captured
/// by `chrome_divergence`; only composition-level `Component::Legend` nodes are.
fn colour_legend_chrome_changes(
    before: &Spec,
    after: &Spec,
    focused_before: &PlotNode,
    focused_after: &PlotNode,
) -> bool {
    let focused_name = plot_name(focused_before);
    // A single within-plot edit adds or removes no legends, so the before spec's
    // legend set is authoritative; only the colour-plot COUNT and the focused
    // plot's own colour-encoding can move.
    let placed_before = count_colour_encoded_plots(before) == 1;
    let placed_after = count_colour_encoded_plots(after) == 1;
    let focused_is_colour =
        plot_is_colour_encoded(focused_before) || plot_is_colour_encoded(focused_after);
    let mut changed = false;
    for_each_legend(before.root.as_ref(), &mut |legend| {
        if legend.channel != LegendChannel::Color {
            return;
        }
        match legend.options.get("for") {
            Some(ValueOrParamRef::Value(SpecValue::String(name))) => {
                if Some(name.as_str()) == focused_name {
                    changed = true;
                }
            }
            None => {
                if placed_before != placed_after || (placed_after && focused_is_colour) {
                    changed = true;
                }
            }
            Some(_) => {}
        }
    });
    changed
}

/// The plot's `name:` attribute, the string a standalone legend's `for:` names
/// it by.
fn plot_name(plot: &PlotNode) -> Option<&str> {
    match plot.attributes.get("name") {
        Some(SpecValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Whether `item` is a colour legend — the item [`ChartEdit::AddColourLegend`]
/// appends and [`ChartEdit::RemoveColourLegend`] takes out.
fn is_colour_legend(item: &Component) -> bool {
    matches!(item, Component::Legend(l) if l.channel == LegendChannel::Color)
}

/// Whether the plot's own items hold a colour legend.
fn holds_colour_legend(plot: &PlotNode) -> bool {
    plot.items.iter().any(is_colour_legend)
}

/// The indices, in item order, of the plot's items that are a colour legend —
/// the items [`ChartEdit::RemoveColourLegend`] takes out.
///
/// Each index is a place in the plot's `plot:` list as the text writes it, for
/// the reason [`mark_item_index`] gives.
#[must_use]
pub fn colour_legend_item_indices(plot: &PlotNode) -> Vec<usize> {
    plot.items
        .iter()
        .enumerate()
        .filter(|(_, c)| is_colour_legend(c))
        .map(|(i, _)| i)
        .collect()
}

/// Whether the plot at `plot_path` already has a colour legend drawn for it, so
/// that [`ChartEdit::AddColourLegend`] would put a second one on the page.
///
/// Two things count: a colour legend among the plot's own items, and a
/// standalone colour legend whose `for:` names the plot's `name:`. A standalone
/// colour legend with no `for:` names no plot and covers none, so a plot beside
/// one is given its own. A `for:` that is a `$param` cannot be resolved from the
/// spec alone, so it covers no plot, and [`classify_edit`] leaves the same
/// `for:` to the reload gate. A path that names no plot is not covered.
#[must_use]
pub fn colour_legend_covers(spec: &Spec, plot_path: &str) -> bool {
    let Some(plot) = plot_at_path(spec, plot_path) else {
        return false;
    };
    if holds_colour_legend(plot) {
        return true;
    }
    let Some(name) = plot_name(plot) else {
        return false;
    };
    collect_legend_nodes(spec).iter().any(|(_, legend)| {
        legend.channel == LegendChannel::Color
            && matches!(
                legend.options.get("for"),
                Some(ValueOrParamRef::Value(SpecValue::String(named))) if named == name
            )
    })
}

/// The plot's colour legends drawn under it, as
/// [`crate::layout::below_legends`] reads them: the path of the `vconcat` that
/// holds the plot and the legends, and each legend's index in it, in order.
/// `None` when no colour legend is drawn under the plot.
///
/// Each legend sits after the plot in the same `vconcat`, so taking one out
/// leaves the plot's index as it was.
fn legends_below(spec: &Spec, plot_path: &str) -> Option<(String, Vec<usize>)> {
    let (parent, _) = plot_path.rsplit_once('/')?;
    let legends: Vec<usize> = below_legends(spec, Rect::new(0.0, 0.0, 0.0, 0.0))
        .into_iter()
        .filter(|below| below.plot_path == plot_path)
        .filter_map(|below| {
            let (holder, step) = below.legend_path.rsplit_once('/')?;
            (holder == parent).then_some(())?;
            step.strip_prefix("vconcat[")?.strip_suffix(']')?.parse().ok()
        })
        .collect();
    (!legends.is_empty()).then(|| (parent.to_string(), legends))
}

/// The index of each colour legend drawn under the plot at `plot_path`, in the
/// `vconcat` that holds the plot: the concat [`plot_route`]'s last step names.
/// `None` when no colour legend is drawn under the plot, which is every
/// placement but [`LegendPlacement::Below`].
#[must_use]
pub fn colour_legends_below(spec: &Spec, plot_path: &str) -> Option<Vec<usize>> {
    legends_below(spec, plot_path).map(|(_, legends)| legends)
}

/// A `name:` for a plot that has none: one no plot of `spec` holds and no
/// legend's `for:` names, so a legend written `for:` it is drawn for that plot
/// alone. `chart`, then `chart-2`, `chart-3` and on.
#[must_use]
pub fn fresh_plot_name(spec: &Spec) -> String {
    let named: Vec<&str> = collect_plot_nodes(spec)
        .into_iter()
        .filter_map(|(_, plot)| plot_name(plot))
        .chain(
            collect_legend_nodes(spec)
                .into_iter()
                .filter_map(|(_, legend)| match legend.options.get("for") {
                    Some(ValueOrParamRef::Value(SpecValue::String(named))) => {
                        Some(named.as_str())
                    }
                    _ => None,
                }),
        )
        .collect();
    let mut n = 1;
    loop {
        let name = if n == 1 {
            "chart".to_string()
        } else {
            format!("chart-{n}")
        };
        if !named.contains(&name.as_str()) {
            return name;
        }
        n += 1;
    }
}

/// The path of the plot `edit` targets once `edit` is applied to `spec`: the
/// path the edit names, but for a [`ChartEdit::PlaceColourLegend`] that moves
/// the plot. A move to below puts the plot first in a new `vconcat` at its
/// place, `root` becoming `root/vconcat[0]`; a move from below that leaves the
/// `vconcat` the plot alone puts the plot in the `vconcat`'s place, and the
/// reverse holds. An edit the reducer would refuse names the path it names.
#[must_use]
pub fn plot_path_after(spec: &Spec, edit: &ChartEdit) -> String {
    let path = edit.plot_path();
    let ChartEdit::PlaceColourLegend { at, .. } = edit else {
        return path.to_string();
    };
    match (legends_below(spec, path), at) {
        (None, LegendPlacement::Below) if plot_at_path(spec, path).is_some() => {
            format!("{path}/vconcat[0]")
        }
        (Some((parent, legends)), LegendPlacement::Right | LegendPlacement::None)
            if concat_len(spec, &parent) == Some(legends.len() + 1) =>
        {
            parent
        }
        _ => path.to_string(),
    }
}

/// The number of entries of the concat at component path `path`.
fn concat_len(spec: &Spec, path: &str) -> Option<usize> {
    match component_at(spec, path)? {
        Component::HConcat(c) | Component::VConcat(c) => Some(c.items.len()),
        _ => None,
    }
}

/// [`ChartEdit::PlaceColourLegend`]'s mutation; its doc says what each move
/// leaves.
fn place_colour_legend(spec: &mut Spec, plot_path: &str, at: LegendPlacement) {
    match (legends_below(spec, plot_path), at) {
        (Some(_), LegendPlacement::Below) => {}
        (Some((parent, legends)), LegendPlacement::Right | LegendPlacement::None) => {
            let Some(Component::VConcat(concat)) = component_at_mut(spec, &parent) else {
                return;
            };
            let options = match &concat.items[legends[0]] {
                Component::Legend(legend) => legend.options.clone(),
                _ => IndexMap::new(),
            };
            for &index in legends.iter().rev() {
                concat.items.remove(index);
            }
            let plot_path = if concat.items.len() == 1 {
                let plot = concat.items.remove(0);
                if let Some(slot) = component_at_mut(spec, &parent) {
                    *slot = plot;
                }
                parent
            } else {
                plot_path.to_string()
            };
            let Some(p) = plot_at_path_mut(spec, &plot_path) else {
                return;
            };
            if at == LegendPlacement::Right {
                if !holds_colour_legend(p) {
                    let mut options = options;
                    options.shift_remove("for");
                    p.items.push(colour_legend(options));
                }
            } else {
                p.items.retain(|c| !is_colour_legend(c));
            }
        }
        (None, LegendPlacement::Below) => {
            let fresh = fresh_plot_name(spec);
            let Some(slot) = component_at_mut(spec, plot_path) else {
                return;
            };
            let Component::Plot(p) = slot else {
                return;
            };
            let carried = p.items.iter().find_map(|c| match c {
                Component::Legend(l) if l.channel == LegendChannel::Color => {
                    Some(l.options.clone())
                }
                _ => None,
            });
            p.items.retain(|c| !is_colour_legend(c));
            let name = match plot_name(p) {
                Some(name) => name.to_string(),
                None => {
                    p.attributes
                        .insert("name".to_string(), SpecValue::String(fresh.clone()));
                    fresh
                }
            };
            let mut options = IndexMap::new();
            options.insert(
                "for".to_string(),
                ValueOrParamRef::Value(SpecValue::String(name)),
            );
            for (key, value) in carried.unwrap_or_default() {
                if key != "for" {
                    options.insert(key, value);
                }
            }
            let plot = std::mem::replace(slot, Component::VConcat(ConcatNode { items: Vec::new() }));
            *slot = Component::VConcat(ConcatNode {
                items: vec![plot, colour_legend(options)],
            });
        }
        (None, LegendPlacement::Right) => {
            if let Some(p) = plot_at_path_mut(spec, plot_path) {
                if !holds_colour_legend(p) {
                    p.items.push(colour_legend(IndexMap::new()));
                }
            }
        }
        (None, LegendPlacement::None) => {
            if let Some(p) = plot_at_path_mut(spec, plot_path) {
                p.items.retain(|c| !is_colour_legend(c));
            }
        }
    }
}

/// A `legend: color` component carrying `options`.
fn colour_legend(options: IndexMap<String, ValueOrParamRef<SpecValue>>) -> Component {
    Component::Legend(LegendNode {
        channel: LegendChannel::Color,
        status: LegendChannel::Color.status(),
        options,
    })
}

/// The component at component path `path` (`root`, `root/hconcat[1]`,
/// `root/hconcat[1]/vconcat[0]`), the path scheme [`plot_at_path`] reads,
/// whatever kind of component it is.
fn component_at<'a>(spec: &'a Spec, path: &str) -> Option<&'a Component> {
    let mut node = spec.root.as_ref()?;
    for step in path.strip_prefix("root")?.split('/').skip(1) {
        let (key, index) = concat_step(step)?;
        node = match (key, node) {
            ("hconcat", Component::HConcat(c)) | ("vconcat", Component::VConcat(c)) => {
                c.items.get(index)?
            }
            _ => return None,
        };
    }
    Some(node)
}

/// Mutable twin of [`component_at`].
fn component_at_mut<'a>(spec: &'a mut Spec, path: &str) -> Option<&'a mut Component> {
    let mut node = spec.root.as_mut()?;
    for step in path.strip_prefix("root")?.split('/').skip(1) {
        let (key, index) = concat_step(step)?;
        node = match (key, node) {
            ("hconcat", Component::HConcat(c)) | ("vconcat", Component::VConcat(c)) => {
                c.items.get_mut(index)?
            }
            _ => return None,
        };
    }
    Some(node)
}

/// One step of a component path, `vconcat[2]`, as its concat key and index.
fn concat_step(step: &str) -> Option<(&str, usize)> {
    let (key, rest) = step.split_once('[')?;
    Some((key, rest.strip_suffix(']')?.parse().ok()?))
}

/// The number of colour-encoded plots in a spec — the count `resolve_legends`
/// keys a no-`for:` colour legend's placement on (exactly one → placed). Delta
/// finding 2.
fn count_colour_encoded_plots(spec: &Spec) -> usize {
    collect_plot_nodes(spec)
        .iter()
        .filter(|(_, plot)| plot_is_colour_encoded(plot))
        .count()
}

/// Visit every standalone [`LegendNode`] under a component subtree (finding 3).
fn for_each_legend(component: Option<&Component>, f: &mut impl FnMut(&LegendNode)) {
    let Some(component) = component else { return };
    match component {
        Component::Legend(l) => f(l),
        Component::Plot(p) => {
            for c in &p.items {
                for_each_legend(Some(c), f);
            }
        }
        Component::HConcat(c) | Component::VConcat(c) => {
            for child in &c.items {
                for_each_legend(Some(child), f);
            }
        }
        _ => {}
    }
}

/// Whether a plot carries a colour encoding (finding 3): any mark binds a colour
/// channel, or any mark's renderer carries a sequential colour scale.
fn plot_is_colour_encoded(plot: &PlotNode) -> bool {
    plot.items.iter().any(|c| match c {
        Component::Mark(m) => {
            kind_carries_colour_scale(m.kind) || m.options.keys().any(|k| is_colour_channel(k))
        }
        _ => false,
    })
}

/// Resolve an axis title DECISION to its concrete text, mirroring the
/// render-side `resolve_axis`: Override -> the string, Suppress -> None, Derive
/// -> the first mark's column for the channel.
fn resolve_derived_title(
    decision: &AxisTitle,
    plot: &PlotNode,
    channel_key: &str,
) -> Option<String> {
    match decision {
        AxisTitle::Override(s) => Some(s.clone()),
        AxisTitle::Suppress => None,
        AxisTitle::Derive => derived_axis_column(plot, channel_key),
    }
}

/// The column a plot's DERIVED x/y axis currently takes its title from: the
/// FIRST mark that binds `channel_key` to a column, mirroring the
/// `resolve_axis` "first map that binds the channel". `None` when no mark binds
/// it (an absent-then-bound rebind still changes the title None -> column).
fn derived_axis_column(plot: &PlotNode, channel_key: &str) -> Option<String> {
    plot.items.iter().find_map(|c| match c {
        Component::Mark(m) => match m.options.get(channel_key) {
            Some(ValueOrParamRef::Value(SpecValue::String(col))) => Some(col.clone()),
            _ => None,
        },
        _ => None,
    })
}

/// Copy only the positional channels ([`INHERITED_CHANNELS`]) from a primary
/// mark's option bag onto an added mark — never a colour channel (see
/// [`classify_edit`]).
fn inherited_positional(
    options: &IndexMap<String, ValueOrParamRef<SpecValue>>,
) -> IndexMap<String, ValueOrParamRef<SpecValue>> {
    let mut out = IndexMap::new();
    for key in INHERITED_CHANNELS {
        if let Some(v) = options.get(*key) {
            out.insert((*key).to_string(), v.clone());
        }
    }
    out
}

/// The axis a mark kind baselines at zero on — a framework-free MIRROR of the
/// render-side `MarkRenderer::zero_baseline_channel` (bar/area/rect value forms,
/// mark.rs) so the classifier can predict the axis-inset flip a retype causes.
/// Every bar/area/rect value form baselines on its OWN value axis: barY/areaY/
/// rectY on y, barX/areaX/rectX on x. Every other mark has no baseline.
///
/// This used to put `BarX` in the `y` arm, mirroring a `BarRenderer` that
/// baselined Y for both orientations. That was the renderer's bug, not a
/// convention, and it is fixed — so barX belongs beside areaX and rectX.
///
/// Keep in sync with the render-side mapping by hand: the cross-crate
/// agreement test that pinned the two retired with the gpui shell.
fn mark_zero_baseline_axis(kind: MarkKind) -> Option<&'static str> {
    match kind {
        MarkKind::BarY | MarkKind::AreaY | MarkKind::RectY => Some("y"),
        MarkKind::BarX | MarkKind::AreaX | MarkKind::RectX => Some("x"),
        _ => None,
    }
}

/// Resolve the item index of the `ordinal`-th MARK in a plot's items (skipping
/// interactors/legends/nested nodes). `None` if the plot has fewer than
/// `ordinal + 1` marks.
fn nth_mark_item_index(plot: &PlotNode, ordinal: usize) -> Option<usize> {
    plot.items
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, Component::Mark(_)))
        .map(|(i, _)| i)
        .nth(ordinal)
}

/// The index in the plot's `plot:` list of the `ordinal`-th mark, which is the
/// mark a [`ChartEdit`] with that `mark_ordinal` targets. `None` if the plot
/// has fewer than `ordinal + 1` marks.
///
/// The index reads as a place in the spec's text because the parser keeps the
/// `plot:` list's items in their list order, a `select:` or a `legend:`
/// holding its index as a mark does.
#[must_use]
pub fn mark_item_index(plot: &PlotNode, ordinal: usize) -> Option<usize> {
    nth_mark_item_index(plot, ordinal)
}

/// Walk the component tree to the plot node identified by `path` (the
/// plot-node path scheme of [`crate::layout::collect_plot_nodes`] /
/// [`crate::analysis::plot_node_path`]: `root`, `root/vconcat[0]`,
/// `root/hconcat[1]/vconcat[0]`, ...). Read-only.
#[must_use]
pub fn plot_at_path<'a>(spec: &'a Spec, path: &str) -> Option<&'a PlotNode> {
    let root = spec.root.as_ref()?;
    descend(root, "root", path)
}

fn descend<'a>(component: &'a Component, here: &str, target: &str) -> Option<&'a PlotNode> {
    match component {
        Component::Plot(p) if here == target => Some(p),
        Component::HConcat(c) => c
            .items
            .iter()
            .enumerate()
            .find_map(|(i, child)| descend(child, &format!("{here}/hconcat[{i}]"), target)),
        Component::VConcat(c) => c
            .items
            .iter()
            .enumerate()
            .find_map(|(i, child)| descend(child, &format!("{here}/vconcat[{i}]"), target)),
        _ => None,
    }
}

/// The route from the spec's root to the plot identified by `path`, as the
/// spec's text nests it: one `(concat key, item index)` step per level, so
/// `root/hconcat[0]/vconcat[1]` is `[("hconcat", 0), ("vconcat", 1)]` and the
/// root plot is the empty route. `None` when `path` names no plot.
///
/// It walks the same tree [`plot_at_path`] walks, and the route reads as a
/// path into the text because the parser keeps a concat's items in their list
/// order — an `hspace` or a `legend` holds its index like a plot does — and
/// because the root component's keys sit at the document root beside `meta:`
/// and `data:`.
#[must_use]
pub fn plot_route(spec: &Spec, path: &str) -> Option<Vec<(&'static str, usize)>> {
    let root = spec.root.as_ref()?;
    let mut route = Vec::new();
    route_to(root, "root", path, &mut route).then_some(route)
}

fn route_to(
    component: &Component,
    here: &str,
    target: &str,
    route: &mut Vec<(&'static str, usize)>,
) -> bool {
    let (key, items) = match component {
        Component::Plot(_) => return here == target,
        Component::HConcat(c) => ("hconcat", &c.items),
        Component::VConcat(c) => ("vconcat", &c.items),
        _ => return false,
    };
    for (i, child) in items.iter().enumerate() {
        route.push((key, i));
        if route_to(child, &format!("{here}/{key}[{i}]"), target, route) {
            return true;
        }
        route.pop();
    }
    false
}

/// Mutable twin of [`plot_at_path`].
#[must_use]
pub fn plot_at_path_mut<'a>(spec: &'a mut Spec, path: &str) -> Option<&'a mut PlotNode> {
    let root = spec.root.as_mut()?;
    descend_mut(root, "root", path)
}

fn descend_mut<'a>(
    component: &'a mut Component,
    here: &str,
    target: &str,
) -> Option<&'a mut PlotNode> {
    match component {
        Component::Plot(p) if here == target => Some(p),
        Component::HConcat(c) => c
            .items
            .iter_mut()
            .enumerate()
            .find_map(|(i, child)| descend_mut(child, &format!("{here}/hconcat[{i}]"), target)),
        Component::VConcat(c) => c
            .items
            .iter_mut()
            .enumerate()
            .find_map(|(i, child)| descend_mut(child, &format!("{here}/vconcat[{i}]"), target)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Snapshot-undo stack with a commit barrier
// ---------------------------------------------------------------------------

/// The result of an [`UndoStack::undo`] request.
#[derive(Debug, Clone, PartialEq)]
pub enum UndoOutcome {
    /// The Spec to restore (the popped pre-edit snapshot).
    Restored(Box<Spec>),
    /// No uncommitted edits remain and none were ever committed — a defined
    /// no-op.
    NothingToUndo,
    /// Every uncommitted edit was already undone and the remaining snapshots
    /// sit BELOW a commit barrier — undo cannot cross a commit (a no-op with a
    /// reason).
    PastCommitBarrier,
}

/// A session snapshot-undo stack: each edit clones the working Spec onto the
/// stack BEFORE `apply`; [`UndoStack::undo`] pops and hands back the snapshot.
/// A commit sets a barrier undo cannot cross. Session-only — no
/// stable ids, not replayable.
#[derive(Debug, Default)]
pub struct UndoStack {
    /// Pre-edit snapshots, oldest first (a stack: newest is `pop`'d first).
    snapshots: Vec<Spec>,
    /// Index below which snapshots are sealed by a commit — undo may only pop
    /// while `snapshots.len() > barrier`.
    barrier: usize,
}

impl UndoStack {
    /// A fresh, empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the pre-edit Spec (call BEFORE [`apply`]).
    pub fn push(&mut self, pre_edit: Spec) {
        self.snapshots.push(pre_edit);
    }

    /// Number of uncommitted edits currently on the stack (edits above the last
    /// commit barrier).
    #[must_use]
    pub fn uncommitted_len(&self) -> usize {
        self.snapshots.len() - self.barrier
    }

    /// Whether there is an uncommitted edit that can be undone.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.snapshots.len() > self.barrier
    }

    /// Pop the most-recent uncommitted snapshot and return it to restore, or a
    /// no-op outcome (empty, or blocked by a commit barrier).
    pub fn undo(&mut self) -> UndoOutcome {
        if self.snapshots.len() > self.barrier {
            // `pop` is Some by the length check.
            UndoOutcome::Restored(Box::new(self.snapshots.pop().expect("non-empty")))
        } else if self.barrier > 0 {
            UndoOutcome::PastCommitBarrier
        } else {
            UndoOutcome::NothingToUndo
        }
    }

    /// Set a commit barrier at the current depth — the accumulated uncommitted
    /// edits are sealed and undo can no longer cross into them.
    pub fn commit_barrier(&mut self) {
        self.barrier = self.snapshots.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyse_spec;
    use crate::parse::{parse_spec, Format};

    fn parse(yaml: &str) -> Spec {
        parse_spec(yaml, Format::Yaml).expect("parse").spec
    }

    fn cp(s: &str) -> ComponentPath {
        ComponentPath(s.to_string())
    }

    const SINGLE: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 'x' AS c
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
";

    // A labelled plot: x/y axes carry explicit xLabel/yLabel, so a rebind is
    // title-STABLE (Override) and therefore gate-clean — the axis on which
    // set-channel is durably supported in v1.
    const SINGLE_LABELLED: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 'x' AS c
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
xLabel: X axis
yLabel: Y axis
";

    const VCONCAT: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
  - plot:
      - mark: line
        data: { from: t }
        x: a
        y: b
";

    fn primary_kind(spec: &Spec, path: &str) -> MarkKind {
        let p = plot_at_path(spec, path).expect("plot");
        p.items
            .iter()
            .find_map(|c| match c {
                Component::Mark(m) => Some(m.kind),
                _ => None,
            })
            .expect("mark")
    }

    fn mark_count(spec: &Spec, path: &str) -> usize {
        plot_at_path(spec, path)
            .expect("plot")
            .items
            .iter()
            .filter(|c| matches!(c, Component::Mark(_)))
            .count()
    }

    // -------- apply mutates the AST exactly per variant --------

    #[test]
    fn change_mark_type_retypes_primary() {
        // dot -> line: a within-zero-baseline-class retype (both non-baseline),
        // so it is gate-clean (a cross-class dot -> bar is refused; see
        // cross_baseline_retype_is_refused).
        let mut spec = parse(SINGLE);
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Dot);
        apply(
            &mut spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Line,
            },
        )
        .expect("clean");
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Line);
    }

    #[test]
    fn set_channel_binds_column() {
        // Rebinding a LABELLED (Override) axis is gate-clean and mutates options.
        let mut spec = parse(SINGLE_LABELLED);
        apply(
            &mut spec,
            &ChartEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "x".to_string(),
                column: "c".to_string(),
            },
        )
        .expect("clean");
        let p = plot_at_path(&spec, "root").unwrap();
        let m = p
            .items
            .iter()
            .find_map(|c| match c {
                Component::Mark(m) => Some(m),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            m.options.get("x"),
            Some(&ValueOrParamRef::Value(SpecValue::String("c".to_string())))
        );
    }

    #[test]
    fn add_mark_appends_and_inherits_data() {
        let mut spec = parse(SINGLE);
        assert_eq!(mark_count(&spec, "root"), 1);
        apply(
            &mut spec,
            &ChartEdit::AddMark {
                plot: cp("root"),
                kind: MarkKind::Line,
            },
        )
        .expect("clean");
        assert_eq!(
            mark_count(&spec, "root"),
            2,
            "AddMark grows the item count by one"
        );
        // The appended mark inherits the primary's data source + x/y.
        let p = plot_at_path(&spec, "root").unwrap();
        let last = p
            .items
            .iter()
            .rev()
            .find_map(|c| match c {
                Component::Mark(m) => Some(m),
                _ => None,
            })
            .unwrap();
        assert_eq!(last.kind, MarkKind::Line);
        assert!(
            last.data.is_some(),
            "added mark inherits the primary's data source"
        );
        assert!(last.options.contains_key("x"), "added mark inherits x");
    }

    #[test]
    fn remove_mark_drops_primary_in_multi_mark_plot() {
        let mut spec = parse(SINGLE);
        apply(
            &mut spec,
            &ChartEdit::AddMark {
                plot: cp("root"),
                kind: MarkKind::Line,
            },
        )
        .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 2);
        apply(
            &mut spec,
            &ChartEdit::RemoveMark {
                plot: cp("root"),
                mark_ordinal: 0,
            },
        )
        .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 1);
        // The remaining primary is the line we added (the dot was removed).
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Line);
    }

    #[test]
    fn gate_tripping_edit_leaves_spec_unchanged() {
        // RemoveMark that would empty the plot: Err, Spec byte-identical.
        let mut spec = parse(SINGLE);
        let before = spec.clone();
        let err = apply(
            &mut spec,
            &ChartEdit::RemoveMark {
                plot: cp("root"),
                mark_ordinal: 0,
            },
        )
        .unwrap_err();
        assert_eq!(err, RefuseReason::WouldEmptyPlot);
        assert_eq!(spec, before, "a refused edit must not mutate the Spec");

        // Rebinding a DERIVED (unlabelled) axis: Err, Spec byte-identical.
        let err = apply(
            &mut spec,
            &ChartEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "x".to_string(),
                column: "c".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err, RefuseReason::WouldChangeAxisTitle);
        assert_eq!(
            spec, before,
            "a refused derived-axis edit must not mutate the Spec"
        );
    }

    #[test]
    fn edits_target_the_focused_plot_in_a_multi_plot_spec() {
        let mut spec = parse(VCONCAT);
        assert_eq!(primary_kind(&spec, "root/vconcat[0]"), MarkKind::Dot);
        assert_eq!(primary_kind(&spec, "root/vconcat[1]"), MarkKind::Line);
        apply(
            &mut spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root/vconcat[1]"),
                mark_ordinal: 0,
                new_kind: MarkKind::Dot,
            },
        )
        .expect("clean");
        // Only the focused plot changed (line -> dot, same zero-baseline class).
        assert_eq!(primary_kind(&spec, "root/vconcat[0]"), MarkKind::Dot);
        assert_eq!(primary_kind(&spec, "root/vconcat[1]"), MarkKind::Dot);
    }

    #[test]
    fn unknown_plot_path_refuses() {
        let mut spec = parse(SINGLE);
        let err = apply(
            &mut spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root/vconcat[9]"),
                mark_ordinal: 0,
                new_kind: MarkKind::BarY,
            },
        )
        .unwrap_err();
        assert_eq!(err, RefuseReason::PlotNotFound);
    }

    // -------- targeting re-walks the live AST (no stale path) ------

    #[test]
    fn remove_then_add_keeps_primary_resolution_correct() {
        // A RemoveMark then AddMark must leave the primary-mark resolution
        // correct — no stale positional path corruption.
        let mut spec = parse(SINGLE);
        apply(
            &mut spec,
            &ChartEdit::AddMark {
                plot: cp("root"),
                kind: MarkKind::Line,
            },
        )
        .expect("clean");
        // Two marks: dot (primary), line.
        apply(
            &mut spec,
            &ChartEdit::RemoveMark {
                plot: cp("root"),
                mark_ordinal: 0,
            },
        )
        .expect("clean");
        // Now line is primary. AddMark a dot (same non-baseline class as line, so
        // gate-clean); the re-walk finds line as primary.
        apply(
            &mut spec,
            &ChartEdit::AddMark {
                plot: cp("root"),
                kind: MarkKind::Dot,
            },
        )
        .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 2);
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Line);
        // Retype the primary once more: still resolves to line, not a stale dot.
        apply(
            &mut spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Rect,
            },
        )
        .expect("clean");
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Rect);
    }

    #[test]
    fn add_mark_yields_two_distinct_nodes_analysis_walks_them() {
        // A second `dot` in one plot stays uniquely addressable by item ordinal
        // (analysis walks item positions, not kind).
        let mut spec = parse(SINGLE);
        apply(
            &mut spec,
            &ChartEdit::AddMark {
                plot: cp("root"),
                kind: MarkKind::Dot,
            },
        )
        .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 2);
        // Analysis still succeeds on the two-dot plot.
        analyse_spec(&spec).expect("analysis on a two-dot plot");
    }

    // -------- snapshot-undo with a commit barrier --------

    #[test]
    fn push_edit_undo_restores_partial_eq() {
        let mut spec = parse(SINGLE);
        let mut undo = UndoStack::new();
        undo.push(spec.clone());
        apply(
            &mut spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Line,
            },
        )
        .expect("clean");
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Line);
        match undo.undo() {
            UndoOutcome::Restored(prev) => spec = *prev,
            other => panic!("expected Restored, got {other:?}"),
        }
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Dot);
    }

    #[test]
    fn three_edits_undo_in_lifo_order() {
        // All retypes stay within the non-zero-baseline class (dot/line/text/rect
        // are all baseline-None), so each is gate-clean.
        let mut spec = parse(SINGLE);
        let mut undo = UndoStack::new();
        for kind in [MarkKind::Line, MarkKind::Text, MarkKind::Rect] {
            undo.push(spec.clone());
            apply(
                &mut spec,
                &ChartEdit::ChangeMarkType {
                    plot: cp("root"),
                    mark_ordinal: 0,
                    new_kind: kind,
                },
            )
            .expect("clean");
        }
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Rect);
        assert_eq!(undo.uncommitted_len(), 3);
        // Undo LIFO: Rect->Text, Text->Line, Line->Dot.
        for expected in [MarkKind::Text, MarkKind::Line, MarkKind::Dot] {
            match undo.undo() {
                UndoOutcome::Restored(prev) => spec = *prev,
                other => panic!("expected Restored, got {other:?}"),
            }
            assert_eq!(primary_kind(&spec, "root"), expected);
        }
    }

    #[test]
    fn undo_cannot_cross_a_commit_barrier() {
        let mut spec = parse(SINGLE);
        let mut undo = UndoStack::new();
        undo.push(spec.clone());
        apply(
            &mut spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Line,
            },
        )
        .expect("clean");
        undo.commit_barrier();
        assert_eq!(undo.uncommitted_len(), 0);
        assert!(!undo.can_undo());
        // Past a commit: a no-op WITH a reason (not NothingToUndo).
        assert_eq!(undo.undo(), UndoOutcome::PastCommitBarrier);
    }

    #[test]
    fn undo_on_empty_stack_is_a_defined_no_op() {
        let mut undo = UndoStack::new();
        assert_eq!(undo.undo(), UndoOutcome::NothingToUndo);
    }

    // -------- gate-classifier verdicts --------

    #[test]
    fn within_plot_edits_are_gate_clean() {
        let spec = parse(SINGLE);
        // A same-class retype (dot -> line) and add are clean (they change no
        // inset baseline / derived title / colour facet).
        assert!(classify_edit(
            &spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Line
            }
        )
        .is_ok());
        assert!(classify_edit(
            &spec,
            &ChartEdit::AddMark {
                plot: cp("root"),
                kind: MarkKind::Line
            }
        )
        .is_ok());
        // Rechannel on a LABELLED (Override) axis is title-stable -> clean.
        let labelled = parse(SINGLE_LABELLED);
        assert!(classify_edit(
            &labelled,
            &ChartEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "x".to_string(),
                column: "c".to_string()
            }
        )
        .is_ok());
    }

    #[test]
    fn rebinding_a_derived_axis_is_refused() {
        // A rebind of a DERIVED (unlabelled) x/y axis changes the axis title,
        // which the launch-fixed margins can't hot-apply — refused.
        let spec = parse(SINGLE);
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::SetChannel {
                    plot: cp("root"),
                    mark_ordinal: 0,
                    channel: "x".to_string(),
                    column: "c".to_string()
                }
            ),
            Err(RefuseReason::WouldChangeAxisTitle)
        );
        // Rebinding to the SAME column it already derives is a no-op title-wise -> clean.
        assert!(classify_edit(
            &spec,
            &ChartEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "x".to_string(),
                column: "a".to_string()
            }
        )
        .is_ok());
    }

    #[test]
    fn cross_baseline_retype_is_refused() {
        // dot (no baseline) -> barY (Y baseline) flips the value-axis inset,
        // launch-fixed chrome -> refused. dot -> circle (both None) is clean.
        let spec = parse(SINGLE);
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::ChangeMarkType {
                    plot: cp("root"),
                    mark_ordinal: 0,
                    new_kind: MarkKind::BarY
                }
            ),
            Err(RefuseReason::WouldChangeInset)
        );
        assert!(classify_edit(
            &spec,
            &ChartEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Circle
            }
        )
        .is_ok());
    }

    #[test]
    fn emptying_a_plot_is_refused() {
        let spec = parse(SINGLE);
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::RemoveMark {
                    plot: cp("root"),
                    mark_ordinal: 0
                }
            ),
            Err(RefuseReason::WouldEmptyPlot)
        );
    }

    #[test]
    fn binding_an_inline_fill_is_clean() {
        // An inline colour fill is NOT captured by chrome_divergence (only
        // STANDALONE legends are), so binding `fill` is gate-clean — a verdict
        // the retired gpui shell's agreement test verified against its gate.
        let spec = parse(SINGLE);
        assert!(classify_edit(
            &spec,
            &ChartEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "fill".to_string(),
                column: "c".to_string()
            }
        )
        .is_ok());
    }

    #[test]
    fn remove_is_clean_when_plot_keeps_a_mark() {
        let mut spec = parse(SINGLE);
        apply(
            &mut spec,
            &ChartEdit::AddMark {
                plot: cp("root"),
                kind: MarkKind::Line,
            },
        )
        .expect("clean");
        assert!(classify_edit(
            &spec,
            &ChartEdit::RemoveMark {
                plot: cp("root"),
                mark_ordinal: 0
            }
        )
        .is_ok());
    }

    // A dashboard with a STANDALONE colour legend `for: scatter` referencing a
    // named plot: a colour edit on that plot changes the legend's scale → the
    // real gate bounces, so the classifier must refuse it (finding 3). The plot
    // is `root/vconcat[1]` (the legend is `root/vconcat[0]`).
    const LEGEND_REFERENCED: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 'x' AS c
vconcat:
  - legend: color
    for: scatter
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
        fill: c
    name: scatter
";

    #[test]
    fn finding3_colour_rebind_under_a_referencing_legend_is_refused() {
        let spec = parse(LEGEND_REFERENCED);
        // A fill rebind on the legend-referenced plot changes its colour scale.
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::SetChannel {
                    plot: cp("root/vconcat[1]"),
                    mark_ordinal: 0,
                    channel: "fill".to_string(),
                    column: "b".to_string(),
                }
            ),
            Err(RefuseReason::WouldChangeLegend)
        );
        // A retype that adds a sequential-colour renderer (dot -> heatmap) is
        // likewise refused under the referencing legend.
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::ChangeMarkType {
                    plot: cp("root/vconcat[1]"),
                    mark_ordinal: 0,
                    new_kind: MarkKind::Heatmap,
                }
            ),
            Err(RefuseReason::WouldChangeLegend)
        );
        // A POSITIONAL (x) rebind on the same plot is NOT a colour change — it is
        // governed by the axis-title rule, not the legend rule (here x is labelled
        // by neither, so it is the derived-title refusal, not the legend one).
        // Bind a labelled axis to isolate: the legend rule must not fire for x.
        assert_ne!(
            classify_edit(
                &spec,
                &ChartEdit::SetChannel {
                    plot: cp("root/vconcat[1]"),
                    mark_ordinal: 0,
                    channel: "x".to_string(),
                    column: "b".to_string(),
                }
            ),
            Err(RefuseReason::WouldChangeLegend),
            "a positional rebind is not a colour-legend change"
        );
    }

    #[test]
    fn finding3_colour_rebind_without_a_legend_stays_clean() {
        // The SAME fill rebind on a plot with NO standalone legend is clean — an
        // inline colour fill is not captured by the gate (the earlier finding).
        let spec = parse(SINGLE);
        assert!(classify_edit(
            &spec,
            &ChartEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "fill".to_string(),
                column: "c".to_string()
            }
        )
        .is_ok());
    }

    // A no-`for:` colour legend + a SINGLE plot that is NOT yet colour-encoded:
    // 0 colour plots -> the legend is unplaced. Adding a fill makes the plot the
    // SOLE colour plot -> the legend appears -> the real gate's `legends` changes
    // (delta finding 2: the 0->1 flip the pre-edit-only check missed).
    const NO_FOR_LEGEND_ONE_PLAIN: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 'x' AS c
vconcat:
  - legend: color
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
    name: scatter
";

    // A no-`for:` colour legend + TWO plots, ONE already colour-encoded (the sole
    // colour plot, so the legend is placed) and one plain. Colouring the plain
    // plot makes TWO colour plots -> the sole legend disappears -> `legends`
    // changes (delta finding 2: the 1->2 flip).
    const NO_FOR_LEGEND_ONE_COLOUR: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b, 'x' AS c
vconcat:
  - legend: color
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
        fill: c
    name: coloured
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
    name: plain
";

    #[test]
    fn finding2_no_for_legend_appears_on_a_zero_to_one_flip_is_refused() {
        // 0 -> 1 colour plots: adding a fill shows the no-`for:` legend. The
        // pre-edit focused plot is NOT colour-encoded, so the old check let this
        // through (the delta-review bug); the count-flip check now refuses it.
        let spec = parse(NO_FOR_LEGEND_ONE_PLAIN);
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::SetChannel {
                    plot: cp("root/vconcat[1]"),
                    mark_ordinal: 0,
                    channel: "fill".to_string(),
                    column: "c".to_string(),
                }
            ),
            Err(RefuseReason::WouldChangeLegend),
            "a 0->1 colour-plot flip shows the no-`for:` legend — refuse"
        );
    }

    #[test]
    fn finding2_no_for_legend_disappears_on_a_one_to_two_flip_is_refused() {
        // 1 -> 2 colour plots: colouring the plain plot hides the sole no-`for:`
        // legend. Again the focused (plain) plot is not colour-encoded pre-edit.
        let spec = parse(NO_FOR_LEGEND_ONE_COLOUR);
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::SetChannel {
                    plot: cp("root/vconcat[2]"),
                    mark_ordinal: 0,
                    channel: "fill".to_string(),
                    column: "c".to_string(),
                }
            ),
            Err(RefuseReason::WouldChangeLegend),
            "a 1->2 colour-plot flip hides the sole no-`for:` legend — refuse"
        );
        // And a rebind on the EXISTING sole colour plot (count stays 1) changes the
        // domain the placed legend renders — refuse (delta finding 2's stays-placed
        // clause).
        assert_eq!(
            classify_edit(
                &spec,
                &ChartEdit::SetChannel {
                    plot: cp("root/vconcat[1]"),
                    mark_ordinal: 0,
                    channel: "fill".to_string(),
                    column: "b".to_string(),
                }
            ),
            Err(RefuseReason::WouldChangeLegend),
            "rebinding the sole colour plot's fill changes the placed legend's domain — refuse"
        );
    }

    #[test]
    fn finding2_no_for_legend_stable_count_is_clean() {
        // A no-`for:` legend with TWO colour plots is UNPLACED (count != 1). A
        // colour rebind on one of them keeps count at 2 -> the legend stays absent
        // -> no `legends` change -> the edit is NOT refused for the legend reason
        // (guards against the conservative-fallback over-refusal).
        let two_colour = "\
data:
  t: SELECT 1 AS a, 2 AS b, 'x' AS c
vconcat:
  - legend: color
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
        fill: c
    name: one
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
        fill: c
    name: two
";
        let spec = parse(two_colour);
        assert_ne!(
            classify_edit(
                &spec,
                &ChartEdit::SetChannel {
                    plot: cp("root/vconcat[1]"),
                    mark_ordinal: 0,
                    channel: "fill".to_string(),
                    column: "b".to_string(),
                }
            ),
            Err(RefuseReason::WouldChangeLegend),
            "a rebind that leaves the unplaced (count 2) legend absent is not a legend change"
        );
    }

    // -------- parse -> apply -> serialise -> re-parse round-trip ----

    #[test]
    fn edited_spec_round_trips_through_the_canonical_serialiser() {
        use crate::parse::serialise_spec;
        // Apply each variant's shape, then round-trip: the re-parsed AST must
        // equal the in-memory edited AST (the commit's re-serialise is lossy on
        // TEXT but idempotent on the AST).
        // Each edit is applied to a fixture on which it is gate-clean (the
        // labelled fixture makes the set-channel rebind title-stable).
        let cases: Vec<(&str, ChartEdit)> = vec![
            (
                SINGLE,
                ChartEdit::ChangeMarkType {
                    plot: cp("root"),
                    mark_ordinal: 0,
                    new_kind: MarkKind::Line,
                },
            ),
            (
                SINGLE_LABELLED,
                ChartEdit::SetChannel {
                    plot: cp("root"),
                    mark_ordinal: 0,
                    channel: "x".to_string(),
                    column: "c".to_string(),
                },
            ),
            (
                SINGLE,
                ChartEdit::AddMark {
                    plot: cp("root"),
                    kind: MarkKind::Line,
                },
            ),
        ];
        for (fixture, edit) in &cases {
            let mut spec = parse(fixture);
            apply(&mut spec, edit).expect("clean edit");
            let yaml = serialise_spec(&spec).expect("serialise");
            let reparsed = parse(&yaml);
            assert_eq!(spec, reparsed, "round-trip AST mismatch for {edit:?}");
        }
    }
}

#[cfg(test)]
mod plot_attribute_tests {
    use super::*;
    use crate::parse::{parse_spec, Format};

    fn parse(yaml: &str) -> Spec {
        parse_spec(yaml, Format::Yaml).expect("parse").spec
    }

    fn cp(s: &str) -> ComponentPath {
        ComponentPath(s.to_string())
    }

    /// Two labelled plots side by side, so an edit aimed at one can be shown
    /// to have left the other alone.
    const PAIR: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b
hconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
    xLabel: X axis
    yLabel: Y axis
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
    xLabel: X axis
    yLabel: Y axis
";

    fn set_scale(path: &str, value: &str) -> ChartEdit {
        ChartEdit::SetPlotAttribute {
            plot: cp(path),
            key: "xScale".to_string(),
            value: SpecValue::String(value.to_string()),
        }
    }

    /// The key lands on the targeted plot, and **only** on it: the sibling
    /// plot's attribute map and items are byte-equal before and after, which
    /// is the half that reddens if the reducer walked to the wrong node or
    /// wrote to every plot it found.
    #[test]
    fn setting_a_plot_attribute_writes_one_key_on_one_plot() {
        let mut spec = parse(PAIR);
        let before = spec.clone();
        apply(&mut spec, &set_scale("root/hconcat[0]", "log")).expect("gate-clean");

        let edited = plot_at_path(&spec, "root/hconcat[0]").expect("plot 0");
        assert_eq!(
            edited.attributes.get("xScale"),
            Some(&SpecValue::String("log".to_string())),
            "the edit writes xScale on the plot it named"
        );
        let was = plot_at_path(&before, "root/hconcat[0]").expect("plot 0 before");
        assert_eq!(
            edited.attributes.len(),
            was.attributes.len() + 1,
            "one key added and nothing else: {:?} -> {:?}",
            was.attributes,
            edited.attributes
        );
        assert_eq!(edited.items, was.items, "no mark on the plot moved");
        assert_eq!(
            plot_at_path(&spec, "root/hconcat[1]"),
            plot_at_path(&before, "root/hconcat[1]"),
            "the sibling plot is untouched"
        );
    }

    /// Writing the key a second time REPLACES it rather than appending, so
    /// switching log -> symlog -> linear leaves one attribute and not three
    /// readings of the same axis.
    #[test]
    fn setting_the_same_attribute_twice_replaces_it() {
        let mut spec = parse(PAIR);
        apply(&mut spec, &set_scale("root/hconcat[0]", "log")).expect("gate-clean");
        let after_one = plot_at_path(&spec, "root/hconcat[0]")
            .expect("plot")
            .attributes
            .len();
        apply(&mut spec, &set_scale("root/hconcat[0]", "symlog")).expect("gate-clean");
        let p = plot_at_path(&spec, "root/hconcat[0]").expect("plot");
        assert_eq!(
            p.attributes.get("xScale"),
            Some(&SpecValue::String("symlog".to_string()))
        );
        assert_eq!(
            p.attributes.len(),
            after_one,
            "the second write replaced the first: {:?}",
            p.attributes
        );
    }

    /// The variant is count-stable and targets no mark, so it reports mark
    /// ordinal 0 and never asks the coordinator for a flat-index rebuild.
    #[test]
    fn a_plot_attribute_edit_is_count_stable() {
        let edit = set_scale("root", "log");
        assert!(!edit.is_count_changing());
        assert_eq!(edit.mark_ordinal(), 0);
        assert_eq!(edit.plot_path(), "root");
        assert_eq!(edit.summary(), "set-plot-attribute: xScale -> log");
    }

    /// An attribute the reload gate watches is refused with the spec
    /// untouched — the generic variant does not become a way around
    /// [`classify_edit`].
    #[test]
    fn an_x_label_written_as_a_plot_attribute_is_refused() {
        let mut spec = parse(PAIR);
        let before = spec.clone();
        let refused = apply(
            &mut spec,
            &ChartEdit::SetPlotAttribute {
                plot: cp("root/hconcat[0]"),
                key: "xLabel".to_string(),
                value: SpecValue::String("Something else".to_string()),
            },
        );
        assert_eq!(refused, Err(RefuseReason::WouldChangeAxisTitle));
        assert_eq!(
            spec, before,
            "a refused edit leaves the spec byte-identical"
        );
    }

    /// A path that names no plot is refused rather than silently dropped.
    #[test]
    fn a_plot_attribute_on_a_missing_plot_is_refused() {
        let mut spec = parse(PAIR);
        let before = spec.clone();
        assert_eq!(
            apply(&mut spec, &set_scale("root/hconcat[9]", "log")),
            Err(RefuseReason::PlotNotFound)
        );
        assert_eq!(spec, before);
    }

    // A plot an analyst put on a log y scale. `yScale` is the FIRST attribute
    // and two follow it, so the order the map keeps is observable: removing a
    // key with `swap_remove` would put `height` where `yScale` was.
    const LOG_Y: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
yScale: log
width: 320
height: 240
";

    // The same plot with no `yScale` key, so a removal of it finds the key absent.
    const LINEAR_Y: &str = "\
data:
  t: SELECT 1 AS a, 2 AS b
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
width: 320
height: 240
";

    // The plot the point-map chart kind writes: a ghost layer, the subset
    // through the shared selection, the brush that publishes it, and the
    // projection at plot level. The map is a spec crate fixture here because
    // the generator lives in `brightfield-shell`, which depends on this crate.
    const HERO_MAP: &str = "\
params:
  brush: { select: crossfilter }
data:
  t: SELECT -122.4 AS longitude, 37.8 AS latitude
plot:
  - mark: dot
    data: { from: t }
    x: longitude
    y: latitude
    fill: \"#cccccc\"
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: longitude
    y: latitude
  - select: intervalXY
    as: $brush
projectionType: equirectangular
width: 640
height: 400
";

    fn remove_attribute(path: &str, key: &str) -> ChartEdit {
        ChartEdit::RemovePlotAttribute {
            plot: cp(path),
            key: key.to_string(),
        }
    }

    fn attribute_keys(spec: &Spec, path: &str) -> Vec<String> {
        plot_at_path(spec, path)
            .expect("plot")
            .attributes
            .keys()
            .cloned()
            .collect()
    }

    /// The key leaves the plot, and **nothing else** does: the marks are
    /// byte-equal and the attributes that were after it keep their order,
    /// which reddens if the map swaps the last key into the hole.
    #[test]
    fn removing_a_plot_attribute_leaves_the_plot_without_the_key() {
        let mut spec = parse(LOG_Y);
        let before = spec.clone();
        assert_eq!(
            attribute_keys(&spec, "root"),
            ["yScale", "width", "height"],
            "the fixture reads its attributes in file order"
        );

        apply(&mut spec, &remove_attribute("root", "yScale")).expect("gate-clean");

        let edited = plot_at_path(&spec, "root").expect("plot");
        assert_eq!(edited.attributes.get("yScale"), None, "the key is gone");
        assert_eq!(
            attribute_keys(&spec, "root"),
            ["width", "height"],
            "the other attributes keep their order"
        );
        assert_eq!(
            edited.attributes.get("width"),
            plot_at_path(&before, "root")
                .expect("plot")
                .attributes
                .get("width"),
            "a neighbouring attribute keeps its value"
        );
        assert_eq!(
            edited.items,
            plot_at_path(&before, "root").expect("plot").items,
            "no mark on the plot moved"
        );
    }

    /// The hero map is drawn through `projectionType`, and a column that is
    /// not a coordinate put on its x needs that key out of the plot. The
    /// removal is gate-clean: a projection changes no axis title or inset the
    /// classifier compares.
    #[test]
    fn removing_the_projection_from_the_hero_map_leaves_a_plot_without_one() {
        let mut spec = parse(HERO_MAP);
        assert_eq!(
            plot_at_path(&spec, "root")
                .expect("plot")
                .attributes
                .get("projectionType"),
            Some(&SpecValue::String("equirectangular".to_string())),
            "the fixture's plot is projected"
        );
        let marks_before = plot_at_path(&spec, "root").expect("plot").items.clone();

        apply(&mut spec, &remove_attribute("root", "projectionType")).expect("gate-clean");

        let edited = plot_at_path(&spec, "root").expect("plot");
        assert_eq!(
            edited.attributes.get("projectionType"),
            None,
            "the map's plot no longer declares a projection"
        );
        assert_eq!(
            attribute_keys(&spec, "root"),
            ["width", "height"],
            "the size the map was drawn at is kept"
        );
        assert_eq!(edited.items, marks_before, "the map's layers are untouched");
    }

    /// A removal of a key the plot never carried is not an error and not a
    /// change: the spec is equal afterwards, including the attributes that
    /// ARE there, which is the half that reddens if the arm clears the map.
    #[test]
    fn removing_an_attribute_the_plot_does_not_carry_leaves_the_spec_equal() {
        let mut spec = parse(LINEAR_Y);
        let before = spec.clone();
        assert_eq!(
            apply(&mut spec, &remove_attribute("root", "yScale")),
            Ok(())
        );
        assert_eq!(spec, before);
        assert_eq!(attribute_keys(&spec, "root"), ["width", "height"]);
    }

    /// One undo on the stack puts the plot back on `yScale: log`: the edit is
    /// bracketed by a snapshot like every other, so the removal is not a
    /// one-way door for the analyst who only tried the scale.
    #[test]
    fn one_undo_after_removing_an_attribute_restores_it() {
        let mut spec = parse(LOG_Y);
        let before = spec.clone();
        let mut undo = UndoStack::new();

        undo.push(spec.clone());
        apply(&mut spec, &remove_attribute("root", "yScale")).expect("gate-clean");
        assert_eq!(
            plot_at_path(&spec, "root")
                .expect("plot")
                .attributes
                .get("yScale"),
            None,
            "the removal happened before the undo"
        );
        assert_eq!(
            undo.uncommitted_len(),
            1,
            "the removal is one undoable edit"
        );

        match undo.undo() {
            UndoOutcome::Restored(restored) => spec = *restored,
            other => panic!("one undo restores the pre-removal spec, got {other:?}"),
        }
        assert_eq!(
            plot_at_path(&spec, "root")
                .expect("plot")
                .attributes
                .get("yScale"),
            Some(&SpecValue::String("log".to_string())),
            "the plot is back on the log scale"
        );
        assert_eq!(spec, before);
        assert_eq!(
            attribute_keys(&spec, "root"),
            ["yScale", "width", "height"],
            "and the key is back where it was"
        );
    }

    /// A removal of `xLabel` is refused with the reason a WRITE of `xLabel` is
    /// refused with, the spec untouched. Dropping the label turns an
    /// overridden (or suppressed) axis title back into one derived from the
    /// column, which grows launch-fixed chrome — so the generic variant is no
    /// way around [`classify_edit`] in this direction either. A plot with no
    /// label to drop is the no-op of the test above, not a refusal.
    #[test]
    fn removing_an_axis_label_is_refused_as_writing_one_is() {
        let mut written = parse(PAIR);
        let write_refusal = apply(
            &mut written,
            &ChartEdit::SetPlotAttribute {
                plot: cp("root/hconcat[0]"),
                key: "xLabel".to_string(),
                value: SpecValue::String("Something else".to_string()),
            },
        );
        assert_eq!(write_refusal, Err(RefuseReason::WouldChangeAxisTitle));

        for (what, yaml, path, key) in [
            ("an overridden x title", PAIR, "root/hconcat[0]", "xLabel"),
            ("an overridden y title", PAIR, "root/hconcat[0]", "yLabel"),
            (
                "a suppressed x title",
                "data:\n  t: SELECT 1 AS a, 2 AS b\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: b\nxLabel: null\n",
                "root",
                "xLabel",
            ),
        ] {
            let mut spec = parse(yaml);
            let before = spec.clone();
            assert_eq!(
                apply(&mut spec, &remove_attribute(path, key)),
                write_refusal,
                "removing {what}"
            );
            assert_eq!(spec, before, "a refused removal of {what} changes nothing");
        }

        let mut unlabelled = parse(LINEAR_Y);
        let before = unlabelled.clone();
        assert_eq!(
            apply(&mut unlabelled, &remove_attribute("root", "xLabel")),
            Ok(())
        );
        assert_eq!(unlabelled, before, "no label to drop, nothing to refuse");
    }

    /// The variant targets no mark and changes no count, so the coordinator
    /// applies it in place and never rebuilds its flat-index maps for it.
    #[test]
    fn a_plot_attribute_removal_is_count_stable() {
        let edit = remove_attribute("root", "yScale");
        assert!(!edit.is_count_changing());
        assert_eq!(edit.mark_ordinal(), 0);
        assert_eq!(edit.plot_path(), "root");
        assert_eq!(edit.kind_name(), "remove-plot-attribute");
        assert_eq!(edit.summary(), "remove-plot-attribute: yScale");
    }
}
