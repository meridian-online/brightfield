//! **The spine's facts**: what each row of the navigator's spine knows before
//! anything draws it.
//!
//! Each step row carries one of five states, what it produced, when it ran
//! within the run and for how long, and where it sits in the graph's gutter.
//! [`crate::protocol::ProtocolModel::spine`] fills them in; this module holds
//! the rules that compute them, so each rule is tested on its own.

use std::collections::BTreeMap;

use brightfield_protocol::contract_graph::SeamStatus;
use brightfield_workbench::subject::RunState;

/// The state a step row shows.
///
/// Five, as the spine's marks have five. A skip is not a state of its own: a
/// skip the record proves current reads as [`SpineState::Ok`], and one it does
/// not prove reads as [`SpineState::NotRun`], because the data behind it is not
/// known to be there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpineState {
    /// No run has made this step's data, or none that can be vouched for.
    NotRun,
    /// The step is running now.
    Running,
    /// The step's data is current: it ran to success, or it was skipped as
    /// current.
    Ok,
    /// The step's last run failed.
    Failed,
    /// The step's data was made before a change to it or to a step above it.
    Stale,
}

/// The state a step row shows, from the step's status in the run and the run
/// state its record proves.
///
/// Running and failed are read off the status first, since a step in flight or
/// one that failed is that whatever came before. A stale record wins over a
/// success, because the success was of a step that has changed since.
#[must_use]
pub fn spine_state(seam: SeamStatus, recorded: RunState) -> SpineState {
    match (seam, recorded) {
        (SeamStatus::Running, _) => SpineState::Running,
        (SeamStatus::Failed, _) | (_, RunState::Failed) => SpineState::Failed,
        (_, RunState::StaleOwnEdit | RunState::StaleUpstream) => SpineState::Stale,
        (SeamStatus::Ok, _) => SpineState::Ok,
        (SeamStatus::Skipped, RunState::Fresh) => SpineState::Ok,
        (SeamStatus::Skipped | SeamStatus::NotRun, _) => SpineState::NotRun,
    }
}

/// Where a row sits in the spine's gutter, which draws the graph's edges down
/// the list's leading edge, one vertical lane per edge in flight.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lanes {
    /// The lane this row's node sits on; `None` on a row with no node, such as
    /// a caption or a view.
    pub node: Option<u8>,
    /// The lanes passing this row without stopping, in lane order.
    pub through: Vec<u8>,
    /// The lanes ending at this row's node, from the rows above it that it is
    /// made from, in lane order.
    pub from_above: Vec<u8>,
    /// The lanes leaving this row's node for the rows below that are made from
    /// it, in the order of the rows they reach.
    pub to_below: Vec<u8>,
}

/// A lane's index as the gutter keeps it. A spine wider than 255 lanes draws
/// its outermost lanes on the last one, which no Protocol a person reads comes
/// near.
fn lane(at: usize) -> u8 {
    u8::try_from(at).unwrap_or(u8::MAX)
}

/// The gutter's lanes for a list of `rows` rows.
///
/// `nodes[r]` is the node row `r` stands for, or `None` for a row with none;
/// `edges` are `(from, to)` pairs of nodes. An edge is drawn down the list
/// from its producer's row to its consumer's, so one whose consumer is not
/// listed below its producer is left out.
///
/// The lanes are assigned in one pass down the list. Each lane is held by one
/// edge from the row it leaves until the row it reaches. A row's node sits on
/// the lowest of the lanes reaching it, or on the lowest free lane when none
/// does, and each edge leaving it takes the lowest lane free at that row, in
/// the order of the rows it reaches. So a chain stays on one lane, a fork opens
/// a lane beside it, and a join closes it again.
#[must_use]
pub fn lanes(rows: usize, nodes: &[Option<usize>], edges: &[(usize, usize)]) -> Vec<Lanes> {
    let mut row_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (row, node) in nodes.iter().enumerate().take(rows) {
        if let Some(node) = node {
            row_of.entry(*node).or_insert(row);
        }
    }
    // Each row's consumers, as rows below it, in row order and once each.
    let mut below: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(from, to) in edges {
        let (Some(&from), Some(&to)) = (row_of.get(&from), row_of.get(&to)) else {
            continue;
        };
        if to > from {
            below.entry(from).or_default().push(to);
        }
    }
    for consumers in below.values_mut() {
        consumers.sort_unstable();
        consumers.dedup();
    }

    // The row each lane is held for, or `None` for a free lane.
    let mut pending: Vec<Option<usize>> = Vec::new();
    let mut out = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut from_above = Vec::new();
        for (at, target) in pending.iter_mut().enumerate() {
            if *target == Some(row) {
                *target = None;
                from_above.push(lane(at));
            }
        }
        let lowest_free = |pending: &[Option<usize>]| {
            pending
                .iter()
                .position(Option::is_none)
                .unwrap_or(pending.len())
        };
        let node = nodes.get(row).copied().flatten().map(|_| {
            from_above
                .first()
                .copied()
                .unwrap_or_else(|| lane(lowest_free(&pending)))
        });
        let through = pending
            .iter()
            .enumerate()
            .filter(|(_, target)| target.is_some())
            .map(|(at, _)| lane(at))
            .collect();
        let mut to_below = Vec::new();
        if node.is_some() {
            for &consumer in below.get(&row).into_iter().flatten() {
                let at = lowest_free(&pending);
                if at == pending.len() {
                    pending.push(Some(consumer));
                } else {
                    pending[at] = Some(consumer);
                }
                to_below.push(lane(at));
            }
        }
        out.push(Lanes {
            node,
            through,
            from_above,
            to_below,
        });
    }
    out
}

/// One step's times, as its run record measured them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepTimes {
    /// When the step started, in milliseconds from the run's start.
    pub start_ms: Option<u64>,
    /// How long it took, in milliseconds.
    pub duration_ms: Option<u64>,
}

/// What a step row shows of when the step ran: its bar on the run's timeline,
/// and how long it took.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// Where the step's bar starts and ends, each a fraction from `0.0` to
    /// `1.0` of the run's span. `None` when the record gives the step no start,
    /// as it does not in a record written before arc measured one, or when the
    /// run took no measurable time.
    pub span: Option<(f32, f32)>,
    /// How long the step took, as [`duration_text`] writes it.
    pub duration: String,
}

/// Each step's [`Timing`], for the steps the record gives a duration.
///
/// The run's span runs from its start to the latest end of a step, a step's
/// end being its start plus its duration. The record's own start and finish
/// are whole seconds, so they are not used for the span.
#[must_use]
pub fn timings(times: &BTreeMap<String, StepTimes>) -> BTreeMap<String, Timing> {
    let span_ms = times
        .values()
        .filter_map(|t| Some(t.start_ms? + t.duration_ms?))
        .max()
        .unwrap_or(0);
    times
        .iter()
        .filter_map(|(step, t)| {
            let duration = t.duration_ms?;
            let span = t.start_ms.filter(|_| span_ms > 0).map(|start| {
                // Milliseconds within a run are drawn as fractions, where an
                // f32's precision is far finer than a pixel.
                #[allow(clippy::cast_precision_loss)]
                let fraction = |ms: u64| (ms as f64 / span_ms as f64) as f32;
                (fraction(start), fraction(start + duration))
            });
            Some((
                step.clone(),
                Timing {
                    span,
                    duration: duration_text(duration),
                },
            ))
        })
        .collect()
}

/// A duration as a step row writes it: seconds to one place under a minute
/// (`2.3s`), then minutes and whole seconds (`1m 04s`), then hours and
/// minutes (`1h 02m`).
#[must_use]
pub fn duration_text(ms: u64) -> String {
    if ms < 59_950 {
        let tenths = (ms + 50) / 100;
        return format!("{}.{}s", tenths / 10, tenths % 10);
    }
    let seconds = (ms + 500) / 1000;
    if seconds < 3600 {
        return format!("{}m {:02}s", seconds / 60, seconds % 60);
    }
    let minutes = (seconds + 30) / 60;
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

/// The fold control a row of a parameterised family carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpineFold {
    /// The id of the family's tile, which a click on the control folds or
    /// unfolds.
    pub family: String,
    /// Whether the family is folded to its one row. An unfolded family carries
    /// the control on the first row of its members.
    pub folded: bool,
    /// How many instances the family repeats.
    pub members: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_sits_on_one_lane() {
        let got = lanes(3, &[Some(0), Some(1), Some(2)], &[(0, 1), (1, 2)]);
        for row in &got {
            assert_eq!(row.node, Some(0));
            assert!(row.through.is_empty(), "{row:?}");
        }
        assert_eq!(got[0].to_below, [0]);
        assert_eq!(got[1].from_above, [0]);
        assert_eq!(got[2].to_below, Vec::<u8>::new());
    }

    #[test]
    fn a_fork_opens_a_second_lane_and_the_join_closes_it() {
        // a → {b, c} → d
        let got = lanes(
            4,
            &[Some(0), Some(1), Some(2), Some(3)],
            &[(0, 1), (0, 2), (1, 3), (2, 3)],
        );
        assert_eq!(got[0].to_below, [0, 1]);
        assert_eq!(got[1].node, Some(0));
        assert_eq!(got[1].through, [1]);
        assert_eq!(got[2].node, Some(1));
        assert_eq!(got[2].through, [0]);
        assert_eq!(got[3].from_above, [0, 1]);
        assert_eq!(got[3].node, Some(0));
        assert!(got[3].through.is_empty());
    }

    #[test]
    fn a_row_with_no_node_carries_the_lanes_through_it() {
        // a, a caption-like row, then b made from a.
        let got = lanes(3, &[Some(0), None, Some(1)], &[(0, 1)]);
        assert_eq!(got[1].node, None);
        assert_eq!(got[1].through, [0]);
        assert_eq!(got[2].from_above, [0]);
    }

    #[test]
    fn an_edge_up_the_list_is_left_out() {
        let got = lanes(2, &[Some(0), Some(1)], &[(1, 0)]);
        assert!(got
            .iter()
            .all(|row| row.to_below.is_empty() && row.from_above.is_empty()));
    }

    #[test]
    fn durations_read_in_seconds_to_one_place() {
        assert_eq!(duration_text(2300), "2.3s");
        assert_eq!(duration_text(40), "0.0s");
        assert_eq!(duration_text(59_949), "59.9s");
        assert_eq!(duration_text(64_000), "1m 04s");
        assert_eq!(duration_text(59_950), "1m 00s");
        assert_eq!(duration_text(3_725_000), "1h 02m");
    }

    #[test]
    fn a_steps_bar_is_its_start_and_end_over_the_runs_span() {
        let times = BTreeMap::from([
            (
                "load".to_string(),
                StepTimes {
                    start_ms: Some(0),
                    duration_ms: Some(1000),
                },
            ),
            (
                "tally".to_string(),
                StepTimes {
                    start_ms: Some(1000),
                    duration_ms: Some(3000),
                },
            ),
            (
                "older".to_string(),
                StepTimes {
                    start_ms: None,
                    duration_ms: Some(500),
                },
            ),
            ("skipped".to_string(), StepTimes::default()),
        ]);
        let got = timings(&times);
        assert_eq!(got["load"].span, Some((0.0, 0.25)));
        assert_eq!(got["tally"].span, Some((0.25, 1.0)));
        assert_eq!(got["tally"].duration, "3.0s");
        assert_eq!(got["older"].span, None, "no start, so no bar");
        assert_eq!(got["older"].duration, "0.5s");
        assert!(!got.contains_key("skipped"), "no duration, so no timing");
    }

    #[test]
    fn a_skip_that_proves_fresh_reads_ok_and_one_that_does_not_reads_not_run() {
        assert_eq!(
            spine_state(SeamStatus::Skipped, RunState::Fresh),
            SpineState::Ok
        );
        assert_eq!(
            spine_state(SeamStatus::Skipped, RunState::NeverRun),
            SpineState::NotRun
        );
        assert_eq!(spine_state(SeamStatus::Ok, RunState::Fresh), SpineState::Ok);
        assert_eq!(
            spine_state(SeamStatus::Ok, RunState::StaleUpstream),
            SpineState::Stale
        );
        assert_eq!(
            spine_state(SeamStatus::Running, RunState::Fresh),
            SpineState::Running
        );
        assert_eq!(
            spine_state(SeamStatus::Failed, RunState::Fresh),
            SpineState::Failed
        );
        assert_eq!(
            spine_state(SeamStatus::NotRun, RunState::NeverRun),
            SpineState::NotRun
        );
    }
}
