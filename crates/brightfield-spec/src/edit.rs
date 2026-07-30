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
use crate::ast::{Component, LegendNode, Mark, PlotNode, Spec, SpecValue, ValueOrParamRef};
use crate::layout::{collect_plot_nodes, resolve_axis_titles, AxisTitle};
use crate::vocab::{LegendChannel, MarkKind};

/// Positional channel keys inherited by an added mark from the plot's primary
/// mark so the new mark actually renders against the same frame (data source +
/// x/y). Only positional channels are inherited (a colour channel would render
/// differently and is a deliberate author choice, not an inheritance).
const INHERITED_CHANNELS: &[&str] = &["x", "y", "x1", "x2", "y1", "y2"];

/// A typed structural mutation applied to the working [`Spec`] by [`apply`] —
/// the framework-free AST-mutation API the keyboard grammar named as missing.
///
/// Four variants (the 5th reserved verb, undo, is an [`UndoStack`] pop, not an
/// edit). Each edit is TYPED (never an exec-string, per the VisiData warning),
/// targets the focused plot's primary mark by walking the live AST via a plot
/// [`ComponentPath`], and is bracketed by a whole-`Spec` clone snapshot so undo
/// is total and near-free. Two variants are count-STABLE
/// ([`ChartEdit::ChangeMarkType`], [`ChartEdit::SetChannel`]) and two are
/// count-CHANGING ([`ChartEdit::AddMark`], [`ChartEdit::RemoveMark`]); the
/// transient apply treats them differently (the coordinator flat-index rebuild).
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
}

impl ChartEdit {
    /// The plot-node path this edit targets.
    #[must_use]
    pub fn plot_path(&self) -> &str {
        match self {
            ChartEdit::ChangeMarkType { plot, .. }
            | ChartEdit::AddMark { plot, .. }
            | ChartEdit::SetChannel { plot, .. }
            | ChartEdit::RemoveMark { plot, .. } => plot.0.as_str(),
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
            ChartEdit::AddMark { .. } => 0,
        }
    }

    /// A short human-readable summary for the command-log panel
    /// (`change-mark-type: -> bar`).
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            ChartEdit::ChangeMarkType { new_kind, .. } => {
                format!("change-mark-type: -> {}", new_kind.wire_name())
            }
            ChartEdit::AddMark { kind, .. } => format!("add-mark: {}", kind.wire_name()),
            ChartEdit::SetChannel {
                channel, column, ..
            } => {
                format!("set-channel: {channel} -> {column}")
            }
            ChartEdit::RemoveMark { .. } => "remove-mark".to_string(),
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

/// Mutate the spec IN PLACE for `edit`, ASSUMING [`classify_edit`]'s structural
/// preconditions already hold (the plot + target mark exist). Never called by
/// the app directly — [`apply`] gates first — but shared with the classifier,
/// which applies it to a CLONE to compute the post-edit chrome signature.
fn apply_unchecked(spec: &mut Spec, edit: &ChartEdit) {
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
    let plot = plot_at_path(spec, edit.plot_path()).ok_or(RefuseReason::PlotNotFound)?;
    let mark_count = plot
        .items
        .iter()
        .filter(|c| matches!(c, Component::Mark(_)))
        .count();

    // Structural preconditions.
    match edit {
        ChartEdit::ChangeMarkType { mark_ordinal, .. }
        | ChartEdit::SetChannel { mark_ordinal, .. }
        | ChartEdit::RemoveMark { mark_ordinal, .. }
            if *mark_ordinal >= mark_count =>
        {
            return Err(RefuseReason::NoSuchMark);
        }
        ChartEdit::RemoveMark { .. } if mark_count <= 1 => {
            return Err(RefuseReason::WouldEmptyPlot);
        }
        _ => {}
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
    let focused_name = focused_before.attributes.get("name").and_then(|v| match v {
        SpecValue::String(s) => Some(s.as_str()),
        _ => None,
    });
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
