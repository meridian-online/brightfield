//! The argument-prompt collector — framework-free.
//!
//! Two of the five keyboard editing verbs take arguments: `add-mark` collects a mark
//! KIND, and `set-channel` collects a CHANNEL then a COLUMN. The Space palette's
//! `run_palette` today only pick->runs a bare action; this gpui-free state
//! machine drives a NEW palette argument-prompt mode. The overlay (the gpui
//! shim) reuses the fuzzy matcher to filter the option lists this collector
//! offers, and feeds each pick back in via [`ArgCollector::pick`]; a completed
//! collection yields the fully-formed [`SpecEdit`] the caller applies.
//!
//! No gpui type crosses this boundary (semantic-layer rule).
//!
//! The palette argument-prompt overlay (shell's `Overlay::Arg`) drives this for
//! the `a`/`e` verbs; the overlay itself is a macOS-eyeball surface, but the
//! collector is unit-tested.

use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::edit::SpecEdit;
use brightfield_spec::vocab::{ImplStatus, MarkKind};

/// The verb an argument overlay is collecting arguments for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgVerb {
    /// `a` — add a mark; collects a KIND.
    AddMark,
    /// `e` — set a channel; collects a CHANNEL then a COLUMN.
    SetChannel,
}

/// What the collector is currently asking for (the overlay's prompt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgStep {
    /// Pick a mark kind (add-mark).
    Kind,
    /// Pick a channel (set-channel, step 1).
    Channel,
    /// Pick a column (set-channel, step 2) — `channel` is already chosen.
    Column {
        /// The channel picked in the previous step.
        channel: String,
    },
}

impl ArgStep {
    /// A short prompt for the overlay ("mark kind", "channel", "column").
    #[must_use]
    pub fn prompt(&self) -> &'static str {
        match self {
            ArgStep::Kind => "mark kind",
            ArgStep::Channel => "channel",
            ArgStep::Column { .. } => "column",
        }
    }
}

/// The outcome of feeding a pick into an [`ArgCollector`].
#[derive(Debug, Clone, PartialEq)]
pub enum ArgOutcome {
    /// Still collecting — the overlay stays open on the (advanced) step.
    Pending,
    /// All arguments collected — apply this edit and close the overlay.
    Ready(SpecEdit),
    /// The pick was not a valid option for the current step — the overlay stays
    /// open (nothing advanced).
    Invalid,
}

/// A running argument collection for one argument-taking verb. Created when the
/// verb fires on a focused plot, advanced by [`ArgCollector::pick`], and dropped
/// (by the overlay setting its slot to `None`) on Esc — the "cancel to Idle"
/// leg.
#[derive(Debug, Clone, PartialEq)]
pub struct ArgCollector {
    plot: ComponentPath,
    mark_ordinal: usize,
    verb: ArgVerb,
    step: ArgStep,
}

impl ArgCollector {
    /// Start collecting for `add-mark` on the focused plot (a single KIND).
    #[must_use]
    pub fn add_mark(plot: ComponentPath) -> Self {
        Self {
            plot,
            mark_ordinal: 0,
            verb: ArgVerb::AddMark,
            step: ArgStep::Kind,
        }
    }

    /// Start collecting for `set-channel` on the focused plot's `mark_ordinal`-th
    /// mark (a CHANNEL then a COLUMN).
    #[must_use]
    pub fn set_channel(plot: ComponentPath, mark_ordinal: usize) -> Self {
        Self {
            plot,
            mark_ordinal,
            verb: ArgVerb::SetChannel,
            step: ArgStep::Channel,
        }
    }

    /// The verb being collected for. A public accessor for parity with
    /// [`Self::step`]; the overlay currently dispatches on `step` alone, so this
    /// is unused in the shim today.
    #[must_use]
    #[allow(dead_code)]
    pub fn verb(&self) -> ArgVerb {
        self.verb
    }

    /// The current step (the overlay's prompt).
    #[must_use]
    pub fn step(&self) -> &ArgStep {
        &self.step
    }

    /// The pick-list the overlay presents for the current step. For a column
    /// step the caller supplies `columns` (from the focused plot's source
    /// profile); the enumerable kind/channel lists ignore it.
    #[must_use]
    pub fn options(&self, columns: &[String]) -> Vec<String> {
        match &self.step {
            ArgStep::Kind => kind_options(),
            ArgStep::Channel => channel_options(),
            ArgStep::Column { .. } => columns.to_vec(),
        }
    }

    /// Feed a pick (a chosen option's canonical string) into the collector,
    /// advancing the step or producing the finished [`SpecEdit`].
    pub fn pick(&mut self, choice: &str) -> ArgOutcome {
        match &self.step {
            ArgStep::Kind => match MarkKind::from_wire(choice) {
                Some(kind) => ArgOutcome::Ready(SpecEdit::AddMark {
                    plot: self.plot.clone(),
                    kind,
                }),
                None => ArgOutcome::Invalid,
            },
            ArgStep::Channel => {
                if channel_options().iter().any(|c| c == choice) {
                    self.step = ArgStep::Column {
                        channel: choice.to_string(),
                    };
                    ArgOutcome::Pending
                } else {
                    ArgOutcome::Invalid
                }
            }
            ArgStep::Column { channel } => {
                if choice.is_empty() {
                    ArgOutcome::Invalid
                } else {
                    ArgOutcome::Ready(SpecEdit::SetChannel {
                        plot: self.plot.clone(),
                        mark_ordinal: self.mark_ordinal,
                        channel: channel.clone(),
                        column: choice.to_string(),
                    })
                }
            }
        }
    }
}

/// The enumerable mark-kind pick-list — the render-able (Implemented) kinds, in
/// vocabulary order, so `add-mark` only offers marks that actually draw.
#[must_use]
pub fn kind_options() -> Vec<String> {
    MarkKind::all()
        .iter()
        .filter(|k| k.status() == ImplStatus::Implemented)
        .map(|k| k.wire_name().to_string())
        .collect()
}

/// The enumerable channel pick-list for `set-channel`. Positional channels lead
/// (the common case); `fill`/`stroke` bind an inline colour encoding (gate-clean
/// — an inline legend is not captured by the reload gate). Note: rebinding a
/// DERIVED (unlabelled) x/y axis is refused-with-reason by the reducer (it would
/// change the launch-fixed axis title), so a plain x/y rebind lands durably only
/// on a labelled axis — surfaced-then-refused, never silently dropped.
#[must_use]
pub fn channel_options() -> Vec<String> {
    ["x", "y", "x1", "x2", "y1", "y2", "fill", "stroke"]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(s: &str) -> ComponentPath {
        ComponentPath(s.to_string())
    }

    #[test]
    fn add_mark_reaches_ready_after_one_kind_pick() {
        let mut c = ArgCollector::add_mark(cp("root"));
        assert_eq!(c.step(), &ArgStep::Kind);
        assert!(c.options(&[]).contains(&"dot".to_string()));
        match c.pick("bar") {
            // "bar" is not a wire name; barY is. So it's invalid.
            ArgOutcome::Invalid => {}
            other => panic!("expected Invalid for a bad kind, got {other:?}"),
        }
        match c.pick("barY") {
            ArgOutcome::Ready(SpecEdit::AddMark { plot, kind }) => {
                assert_eq!(plot, cp("root"));
                assert_eq!(kind, MarkKind::BarY);
            }
            other => panic!("expected Ready(AddMark), got {other:?}"),
        }
    }

    #[test]
    fn set_channel_needs_a_channel_then_a_column() {
        let mut c = ArgCollector::set_channel(cp("root"), 0);
        assert_eq!(c.step(), &ArgStep::Channel);
        assert_eq!(
            c.pick("x"),
            ArgOutcome::Pending,
            "channel pick advances to column"
        );
        assert_eq!(
            c.step(),
            &ArgStep::Column {
                channel: "x".to_string()
            }
        );
        // The column list comes from the caller (source profile).
        let cols = vec!["temp".to_string(), "depth".to_string()];
        assert_eq!(c.options(&cols), cols);
        match c.pick("depth") {
            ArgOutcome::Ready(SpecEdit::SetChannel {
                plot,
                mark_ordinal,
                channel,
                column,
            }) => {
                assert_eq!(plot, cp("root"));
                assert_eq!(mark_ordinal, 0);
                assert_eq!(channel, "x");
                assert_eq!(column, "depth");
            }
            other => panic!("expected Ready(SetChannel), got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_channel_is_invalid() {
        let mut c = ArgCollector::set_channel(cp("root"), 0);
        assert_eq!(c.pick("wobble"), ArgOutcome::Invalid);
        assert_eq!(
            c.step(),
            &ArgStep::Channel,
            "an invalid pick does not advance"
        );
    }

    #[test]
    fn fill_is_offered_as_a_channel() {
        // fill/stroke bind an inline colour encoding — offered and gate-clean.
        assert!(channel_options().contains(&"fill".to_string()));
        assert!(channel_options().contains(&"stroke".to_string()));
    }
}
