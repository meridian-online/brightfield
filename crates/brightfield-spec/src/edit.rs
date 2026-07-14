//! The SpecEdit spine — typed structural mutations of the working Spec (card
//! 0023, the keyboard command-log).
//!
//! Card 0018's keyboard grammar named five reserved verbs (`m` / `a` / `e` /
//! `d` / undo) that all "need the command log": a way to change a mark's type,
//! add a mark, bind a channel, remove a mark, and undo — applied live, then
//! committed on a deliberate action. This module is the substrate behind them:
//! a framework-free [`SpecEdit`] enum + [`apply`] reducer that walks the root
//! [`Component`] tree by focused-plot path and mutates the AST in place, a
//! snapshot [`UndoStack`] with a commit barrier, and a [`classify_edit`]
//! gate-classifier that REIMPLEMENTS the app-binary reload gate
//! (`same_layout` / `chrome_divergence`) from the spec representation, so a
//! within-plot edit that WOULD bounce to "restart to apply" is refused at edit
//! time with a reason instead.
//!
//! No gpui type crosses this boundary — the app layer is the shim that drives
//! the reducer and re-renders (the standing framework-free rule, mirroring
//! `brightfield-keys` / `spec_save`). Targeting is by focused-plot + ordinal
//! (v1: the plot's PRIMARY/first mark): count-changing edits re-walk the live
//! AST every time and never cache a positional path string, so a within-plot
//! insert/remove that renumbers later siblings can't corrupt a stored path.

use indexmap::IndexMap;

use crate::analysis::ComponentPath;
use crate::ast::{Component, Mark, PlotNode, Spec, SpecValue, ValueOrParamRef};
use crate::layout::{resolve_axis_titles, AxisTitle};
use crate::vocab::MarkKind;

/// Positional channel keys inherited by an added mark from the plot's primary
/// mark so the new mark actually renders against the same frame (data source +
/// x/y). Only positional channels are inherited (a colour channel would render
/// differently and is a deliberate author choice, not an inheritance).
const INHERITED_CHANNELS: &[&str] = &["x", "y", "x1", "x2", "y1", "y2"];

/// A typed structural mutation applied to the working [`Spec`] by [`apply`] —
/// the gpui-free AST-mutation API card 0018 named as missing.
///
/// Four variants (the 5th reserved verb, undo, is an [`UndoStack`] pop, not an
/// edit). Each edit is TYPED (never an exec-string, per the VisiData warning),
/// targets the focused plot's primary mark by walking the live AST via a plot
/// [`ComponentPath`], and is bracketed by a whole-`Spec` clone snapshot so undo
/// is total and near-free. Two variants are count-STABLE
/// ([`SpecEdit::ChangeMarkType`], [`SpecEdit::SetChannel`]) and two are
/// count-CHANGING ([`SpecEdit::AddMark`], [`SpecEdit::RemoveMark`]); the
/// transient apply treats them differently (the coordinator flat-index rebuild,
/// clg-ac16).
#[derive(Debug, Clone, PartialEq)]
pub enum SpecEdit {
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

impl SpecEdit {
    /// The plot-node path this edit targets.
    #[must_use]
    pub fn plot_path(&self) -> &str {
        match self {
            SpecEdit::ChangeMarkType { plot, .. }
            | SpecEdit::AddMark { plot, .. }
            | SpecEdit::SetChannel { plot, .. }
            | SpecEdit::RemoveMark { plot, .. } => plot.0.as_str(),
        }
    }

    /// Whether this edit changes the mark COUNT (AddMark / RemoveMark) — the
    /// transient apply must rebuild the coordinator + engine flat-index maps
    /// for a count-changing edit (clg-ac16).
    #[must_use]
    pub fn is_count_changing(&self) -> bool {
        matches!(self, SpecEdit::AddMark { .. } | SpecEdit::RemoveMark { .. })
    }

    /// A short human-readable summary for the command-log panel
    /// (`change-mark-type: -> bar`).
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            SpecEdit::ChangeMarkType { new_kind, .. } => {
                format!("change-mark-type: -> {}", new_kind.wire_name())
            }
            SpecEdit::AddMark { kind, .. } => format!("add-mark: {}", kind.wire_name()),
            SpecEdit::SetChannel { channel, column, .. } => {
                format!("set-channel: {channel} -> {column}")
            }
            SpecEdit::RemoveMark { .. } => "remove-mark".to_string(),
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
    /// Rebinding a DERIVED x/y axis would change the axis title (card 0019
    /// derives the title from the encoding's column name), which grows the
    /// launch-fixed margins — a `chrome_divergence` a reload can't hot-apply. v1
    /// refuses it; bind such an axis on a plot with an explicit `xLabel`/`yLabel`
    /// (an Override / Suppress axis is title-stable under a rebind).
    WouldChangeAxisTitle,
    /// Retyping to a mark of a DIFFERENT zero-baseline class (e.g. `dot` -> `bar`)
    /// would flip the axis-inset default on the value axis (card 0008 — a
    /// zero-baseline end stays flush), a launch-fixed `chrome_divergence`. v1
    /// allows a retype only WITHIN the same zero-baseline class (dot<->line,
    /// barY<->areaY<->rectY, ...).
    WouldChangeInset,
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
        }
    }
}

/// Apply a structural edit to the working Spec IN PLACE, or return
/// `Err(RefuseReason)` WITHOUT mutating when the edit would trip a reload gate
/// or its target does not exist (clg-ac01).
///
/// The classifier runs first ([`classify_edit`]) so a gate-tripping edit never
/// mutates; the caller snapshots the pre-edit Spec BEFORE calling apply and, on
/// `Err`, discards the snapshot (nothing changed).
pub fn apply(spec: &mut Spec, edit: &SpecEdit) -> Result<(), RefuseReason> {
    // Gate FIRST — a refused edit must leave the Spec byte-identical.
    classify_edit(spec, edit)?;
    apply_unchecked(spec, edit);
    Ok(())
}

/// Mutate the spec IN PLACE for `edit`, ASSUMING [`classify_edit`]'s structural
/// preconditions already hold (the plot + target mark exist). Never called by
/// the app directly — [`apply`] gates first — but shared with the classifier,
/// which applies it to a CLONE to compute the post-edit chrome signature.
fn apply_unchecked(spec: &mut Spec, edit: &SpecEdit) {
    let Some(p) = plot_at_path_mut(spec, edit.plot_path()) else {
        return;
    };
    match edit {
        SpecEdit::ChangeMarkType { mark_ordinal, new_kind, .. } => {
            if let Some(item) = nth_mark_item_index(p, *mark_ordinal) {
                if let Component::Mark(m) = &mut p.items[item] {
                    m.kind = *new_kind;
                    m.status = new_kind.status();
                }
            }
        }
        SpecEdit::AddMark { kind, .. } => {
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
        SpecEdit::SetChannel { mark_ordinal, channel, column, .. } => {
            if let Some(item) = nth_mark_item_index(p, *mark_ordinal) {
                if let Component::Mark(m) = &mut p.items[item] {
                    m.options.insert(
                        channel.clone(),
                        ValueOrParamRef::Value(SpecValue::String(column.clone())),
                    );
                }
            }
        }
        SpecEdit::RemoveMark { mark_ordinal, .. } => {
            if let Some(item) = nth_mark_item_index(p, *mark_ordinal) {
                p.items.remove(item);
            }
        }
    }
}

/// Classify whether a pending edit would trip a reload gate or has no valid
/// target — WITHOUT mutating (clg-ac11). This REIMPLEMENTS the app-binary reload
/// gate (`same_layout` / `chrome_divergence`, main.rs) from the spec
/// representation, because brightfield-app has no `[lib]` target; a brightfield-
/// app AGREEMENT test pins these verdicts equal.
///
/// Two structural preconditions refuse first (a missing target mark; a
/// `RemoveMark` that would EMPTY the plot). Then the WITHIN-PLOT chrome signature
/// is diffed BEFORE vs AFTER the edit (applied to a clone): the axis-inset
/// baseline SET (card 0008 — an axis end is flush iff ANY mark zero-baselines it)
/// and the DERIVED x/y axis titles (card 0019 — a Derive axis takes the first
/// mark's column). A difference is refused ([`RefuseReason::WouldChangeInset`] /
/// [`RefuseReason::WouldChangeAxisTitle`]) because both feed launch-fixed chrome
/// a reload can't hot-apply. Everything a within-plot mark edit CANNOT change is
/// gate-clean and needs no check: plot count/geometry (`same_layout`), the
/// colorScheme, the dashboard title, standalone legends, inline-legend
/// suppression — so binding an inline `fill` (NOT captured by the gate) is
/// allowed. The inset check is CONSERVATIVE on a categorical axis (the gate
/// applies no inset default there, so a baseline flip is inert): it may
/// over-refuse, which is the safe side (never a silent bounce).
pub fn classify_edit(spec: &Spec, edit: &SpecEdit) -> Result<(), RefuseReason> {
    let plot = plot_at_path(spec, edit.plot_path()).ok_or(RefuseReason::PlotNotFound)?;
    let mark_count = plot.items.iter().filter(|c| matches!(c, Component::Mark(_))).count();

    // Structural preconditions.
    match edit {
        SpecEdit::ChangeMarkType { mark_ordinal, .. }
        | SpecEdit::SetChannel { mark_ordinal, .. }
        | SpecEdit::RemoveMark { mark_ordinal, .. }
            if *mark_ordinal >= mark_count =>
        {
            return Err(RefuseReason::NoSuchMark);
        }
        SpecEdit::RemoveMark { .. } if mark_count <= 1 => {
            return Err(RefuseReason::WouldEmptyPlot);
        }
        _ => {}
    }

    // Chrome-signature comparison: apply to a clone and diff the launch-fixed
    // chrome the reload gate compares.
    let before = plot_chrome_signature(plot);
    let mut clone = spec.clone();
    apply_unchecked(&mut clone, edit);
    let after_plot = plot_at_path(&clone, edit.plot_path()).ok_or(RefuseReason::PlotNotFound)?;
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

/// Resolve an axis title DECISION to its concrete text, mirroring card 0019's
/// render-side `resolve_axis`: Override -> the string, Suppress -> None, Derive
/// -> the first mark's column for the channel.
fn resolve_derived_title(decision: &AxisTitle, plot: &PlotNode, channel_key: &str) -> Option<String> {
    match decision {
        AxisTitle::Override(s) => Some(s.clone()),
        AxisTitle::Suppress => None,
        AxisTitle::Derive => derived_axis_column(plot, channel_key),
    }
}

/// The column a plot's DERIVED x/y axis currently takes its title from: the
/// FIRST mark that binds `channel_key` to a column, mirroring card 0019's
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

/// The axis a mark kind baselines at zero on — a gpui-free MIRROR of the
/// render-side `MarkRenderer::zero_baseline_channel` (bar/area/rect value forms,
/// mark.rs) so the classifier can predict the axis-inset flip a retype causes
/// (card 0008). `BarRenderer` baselines Y for BOTH barX and barY; area/rect
/// value forms baseline their value axis; every other mark has no baseline.
/// Pinned to the real renderer mapping by the brightfield-app agreement test.
fn mark_zero_baseline_axis(kind: MarkKind) -> Option<&'static str> {
    match kind {
        MarkKind::BarX | MarkKind::BarY | MarkKind::AreaY | MarkKind::RectY => Some("y"),
        MarkKind::AreaX | MarkKind::RectX => Some("x"),
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
        Component::HConcat(c) => c.items.iter().enumerate().find_map(|(i, child)| {
            descend(child, &format!("{here}/hconcat[{i}]"), target)
        }),
        Component::VConcat(c) => c.items.iter().enumerate().find_map(|(i, child)| {
            descend(child, &format!("{here}/vconcat[{i}]"), target)
        }),
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
        Component::HConcat(c) => c.items.iter_mut().enumerate().find_map(|(i, child)| {
            descend_mut(child, &format!("{here}/hconcat[{i}]"), target)
        }),
        Component::VConcat(c) => c.items.iter_mut().enumerate().find_map(|(i, child)| {
            descend_mut(child, &format!("{here}/vconcat[{i}]"), target)
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Snapshot-undo stack with a commit barrier (clg-ac02)
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
/// A commit sets a barrier undo cannot cross (clg-ac02). Session-only — no
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

    // -------- clg-ac01: apply mutates the AST exactly per variant --------

    #[test]
    fn clg_ac01_change_mark_type_retypes_primary() {
        // dot -> line: a within-zero-baseline-class retype (both non-baseline),
        // so it is gate-clean (a cross-class dot -> bar is refused; see
        // clg_ac11_cross_baseline_retype_is_refused).
        let mut spec = parse(SINGLE);
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Dot);
        apply(
            &mut spec,
            &SpecEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Line,
            },
        )
        .expect("clean");
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Line);
    }

    #[test]
    fn clg_ac01_set_channel_binds_column() {
        // Rebinding a LABELLED (Override) axis is gate-clean and mutates options.
        let mut spec = parse(SINGLE_LABELLED);
        apply(
            &mut spec,
            &SpecEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "x".to_string(),
                column: "c".to_string(),
            },
        )
        .expect("clean");
        let p = plot_at_path(&spec, "root").unwrap();
        let m = p.items.iter().find_map(|c| match c {
            Component::Mark(m) => Some(m),
            _ => None,
        }).unwrap();
        assert_eq!(
            m.options.get("x"),
            Some(&ValueOrParamRef::Value(SpecValue::String("c".to_string())))
        );
    }

    #[test]
    fn clg_ac01_add_mark_appends_and_inherits_data() {
        let mut spec = parse(SINGLE);
        assert_eq!(mark_count(&spec, "root"), 1);
        apply(&mut spec, &SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Line })
            .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 2, "AddMark grows the item count by one");
        // The appended mark inherits the primary's data source + x/y.
        let p = plot_at_path(&spec, "root").unwrap();
        let last = p.items.iter().rev().find_map(|c| match c {
            Component::Mark(m) => Some(m),
            _ => None,
        }).unwrap();
        assert_eq!(last.kind, MarkKind::Line);
        assert!(last.data.is_some(), "added mark inherits the primary's data source");
        assert!(last.options.contains_key("x"), "added mark inherits x");
    }

    #[test]
    fn clg_ac01_remove_mark_drops_primary_in_multi_mark_plot() {
        let mut spec = parse(SINGLE);
        apply(&mut spec, &SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Line })
            .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 2);
        apply(&mut spec, &SpecEdit::RemoveMark { plot: cp("root"), mark_ordinal: 0 })
            .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 1);
        // The remaining primary is the line we added (the dot was removed).
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Line);
    }

    #[test]
    fn clg_ac01_gate_tripping_edit_leaves_spec_unchanged() {
        // RemoveMark that would empty the plot: Err, Spec byte-identical.
        let mut spec = parse(SINGLE);
        let before = spec.clone();
        let err = apply(&mut spec, &SpecEdit::RemoveMark { plot: cp("root"), mark_ordinal: 0 })
            .unwrap_err();
        assert_eq!(err, RefuseReason::WouldEmptyPlot);
        assert_eq!(spec, before, "a refused edit must not mutate the Spec");

        // Rebinding a DERIVED (unlabelled) axis: Err, Spec byte-identical.
        let err = apply(
            &mut spec,
            &SpecEdit::SetChannel {
                plot: cp("root"),
                mark_ordinal: 0,
                channel: "x".to_string(),
                column: "c".to_string(),
            },
        )
        .unwrap_err();
        assert_eq!(err, RefuseReason::WouldChangeAxisTitle);
        assert_eq!(spec, before, "a refused derived-axis edit must not mutate the Spec");
    }

    #[test]
    fn clg_ac01_edits_target_the_focused_plot_in_a_multi_plot_spec() {
        let mut spec = parse(VCONCAT);
        assert_eq!(primary_kind(&spec, "root/vconcat[0]"), MarkKind::Dot);
        assert_eq!(primary_kind(&spec, "root/vconcat[1]"), MarkKind::Line);
        apply(
            &mut spec,
            &SpecEdit::ChangeMarkType {
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
    fn clg_ac01_unknown_plot_path_refuses() {
        let mut spec = parse(SINGLE);
        let err = apply(
            &mut spec,
            &SpecEdit::ChangeMarkType {
                plot: cp("root/vconcat[9]"),
                mark_ordinal: 0,
                new_kind: MarkKind::BarY,
            },
        )
        .unwrap_err();
        assert_eq!(err, RefuseReason::PlotNotFound);
    }

    // -------- clg-ac04: targeting re-walks the live AST (no stale path) ------

    #[test]
    fn clg_ac04_remove_then_add_keeps_primary_resolution_correct() {
        // A RemoveMark then AddMark must leave the primary-mark resolution
        // correct — no stale positional path corruption.
        let mut spec = parse(SINGLE);
        apply(&mut spec, &SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Line })
            .expect("clean");
        // Two marks: dot (primary), line.
        apply(&mut spec, &SpecEdit::RemoveMark { plot: cp("root"), mark_ordinal: 0 })
            .expect("clean");
        // Now line is primary. AddMark a dot (same non-baseline class as line, so
        // gate-clean); the re-walk finds line as primary.
        apply(&mut spec, &SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Dot })
            .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 2);
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Line);
        // Retype the primary once more: still resolves to line, not a stale dot.
        apply(
            &mut spec,
            &SpecEdit::ChangeMarkType {
                plot: cp("root"),
                mark_ordinal: 0,
                new_kind: MarkKind::Rect,
            },
        )
        .expect("clean");
        assert_eq!(primary_kind(&spec, "root"), MarkKind::Rect);
    }

    #[test]
    fn clg_ac04_add_mark_yields_two_distinct_nodes_analysis_walks_them() {
        // A second `dot` in one plot stays uniquely addressable by item ordinal
        // (analysis walks item positions, not kind).
        let mut spec = parse(SINGLE);
        apply(&mut spec, &SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Dot })
            .expect("clean");
        assert_eq!(mark_count(&spec, "root"), 2);
        // Analysis still succeeds on the two-dot plot.
        analyse_spec(&spec).expect("analysis on a two-dot plot");
    }

    // -------- clg-ac02: snapshot-undo with a commit barrier --------

    #[test]
    fn clg_ac02_push_edit_undo_restores_partial_eq() {
        let mut spec = parse(SINGLE);
        let mut undo = UndoStack::new();
        undo.push(spec.clone());
        apply(
            &mut spec,
            &SpecEdit::ChangeMarkType {
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
    fn clg_ac02_three_edits_undo_in_lifo_order() {
        // All retypes stay within the non-zero-baseline class (dot/line/text/rect
        // are all baseline-None), so each is gate-clean.
        let mut spec = parse(SINGLE);
        let mut undo = UndoStack::new();
        for kind in [MarkKind::Line, MarkKind::Text, MarkKind::Rect] {
            undo.push(spec.clone());
            apply(
                &mut spec,
                &SpecEdit::ChangeMarkType { plot: cp("root"), mark_ordinal: 0, new_kind: kind },
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
    fn clg_ac02_undo_cannot_cross_a_commit_barrier() {
        let mut spec = parse(SINGLE);
        let mut undo = UndoStack::new();
        undo.push(spec.clone());
        apply(
            &mut spec,
            &SpecEdit::ChangeMarkType { plot: cp("root"), mark_ordinal: 0, new_kind: MarkKind::Line },
        )
        .expect("clean");
        undo.commit_barrier();
        assert_eq!(undo.uncommitted_len(), 0);
        assert!(!undo.can_undo());
        // Past a commit: a no-op WITH a reason (not NothingToUndo).
        assert_eq!(undo.undo(), UndoOutcome::PastCommitBarrier);
    }

    #[test]
    fn clg_ac02_undo_on_empty_stack_is_a_defined_no_op() {
        let mut undo = UndoStack::new();
        assert_eq!(undo.undo(), UndoOutcome::NothingToUndo);
    }

    // -------- clg-ac11: gate-classifier verdicts --------

    #[test]
    fn clg_ac11_within_plot_edits_are_gate_clean() {
        let spec = parse(SINGLE);
        // A same-class retype (dot -> line) and add are clean (they change no
        // inset baseline / derived title / colour facet).
        assert!(classify_edit(
            &spec,
            &SpecEdit::ChangeMarkType { plot: cp("root"), mark_ordinal: 0, new_kind: MarkKind::Line }
        )
        .is_ok());
        assert!(classify_edit(&spec, &SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Line }).is_ok());
        // Rechannel on a LABELLED (Override) axis is title-stable -> clean.
        let labelled = parse(SINGLE_LABELLED);
        assert!(classify_edit(
            &labelled,
            &SpecEdit::SetChannel { plot: cp("root"), mark_ordinal: 0, channel: "x".to_string(), column: "c".to_string() }
        )
        .is_ok());
    }

    #[test]
    fn clg_ac11_rebinding_a_derived_axis_is_refused() {
        // A rebind of a DERIVED (unlabelled) x/y axis changes the axis title
        // (card 0019), which the launch-fixed margins can't hot-apply — refused.
        let spec = parse(SINGLE);
        assert_eq!(
            classify_edit(
                &spec,
                &SpecEdit::SetChannel { plot: cp("root"), mark_ordinal: 0, channel: "x".to_string(), column: "c".to_string() }
            ),
            Err(RefuseReason::WouldChangeAxisTitle)
        );
        // Rebinding to the SAME column it already derives is a no-op title-wise -> clean.
        assert!(classify_edit(
            &spec,
            &SpecEdit::SetChannel { plot: cp("root"), mark_ordinal: 0, channel: "x".to_string(), column: "a".to_string() }
        )
        .is_ok());
    }

    #[test]
    fn clg_ac11_cross_baseline_retype_is_refused() {
        // dot (no baseline) -> barY (Y baseline) flips the value-axis inset,
        // launch-fixed chrome -> refused. dot -> circle (both None) is clean.
        let spec = parse(SINGLE);
        assert_eq!(
            classify_edit(
                &spec,
                &SpecEdit::ChangeMarkType { plot: cp("root"), mark_ordinal: 0, new_kind: MarkKind::BarY }
            ),
            Err(RefuseReason::WouldChangeInset)
        );
        assert!(classify_edit(
            &spec,
            &SpecEdit::ChangeMarkType { plot: cp("root"), mark_ordinal: 0, new_kind: MarkKind::Circle }
        )
        .is_ok());
    }

    #[test]
    fn clg_ac11_emptying_a_plot_is_refused() {
        let spec = parse(SINGLE);
        assert_eq!(
            classify_edit(&spec, &SpecEdit::RemoveMark { plot: cp("root"), mark_ordinal: 0 }),
            Err(RefuseReason::WouldEmptyPlot)
        );
    }

    #[test]
    fn clg_ac11_binding_an_inline_fill_is_clean() {
        // An inline colour fill is NOT captured by chrome_divergence (only
        // STANDALONE legends are), so binding `fill` is gate-clean — verified
        // against the real gate by the brightfield-app agreement test.
        let spec = parse(SINGLE);
        assert!(classify_edit(
            &spec,
            &SpecEdit::SetChannel { plot: cp("root"), mark_ordinal: 0, channel: "fill".to_string(), column: "c".to_string() }
        )
        .is_ok());
    }

    #[test]
    fn clg_ac11_remove_is_clean_when_plot_keeps_a_mark() {
        let mut spec = parse(SINGLE);
        apply(&mut spec, &SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Line })
            .expect("clean");
        assert!(classify_edit(&spec, &SpecEdit::RemoveMark { plot: cp("root"), mark_ordinal: 0 }).is_ok());
    }

    // -------- clg-ac07a: parse -> apply -> serialise -> re-parse round-trip ----

    #[test]
    fn clg_ac07a_edited_spec_round_trips_through_the_canonical_serialiser() {
        use crate::parse::serialise_spec;
        // Apply each variant's shape, then round-trip: the re-parsed AST must
        // equal the in-memory edited AST (the commit's re-serialise is lossy on
        // TEXT but idempotent on the AST).
        // Each edit is applied to a fixture on which it is gate-clean (the
        // labelled fixture makes the set-channel rebind title-stable).
        let cases: Vec<(&str, SpecEdit)> = vec![
            (SINGLE, SpecEdit::ChangeMarkType { plot: cp("root"), mark_ordinal: 0, new_kind: MarkKind::Line }),
            (SINGLE_LABELLED, SpecEdit::SetChannel { plot: cp("root"), mark_ordinal: 0, channel: "x".to_string(), column: "c".to_string() }),
            (SINGLE, SpecEdit::AddMark { plot: cp("root"), kind: MarkKind::Line }),
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
