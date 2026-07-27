//! Pan and zoom: the gesture arithmetic, the axis lock, and the rule that
//! decides when a moving frame is allowed to become a query.
//!
//! Framework-free and pure — no egui type crosses this boundary, and every
//! function here is driven directly by unit tests as well as through the chart
//! pane that calls it.
//!
//! # The frame moves continuously; the data re-queries once
//!
//! A navigation gesture is not a brush. A brush produces one predicate when it
//! is released; a pan produces a new frame on every pointer sample, and a wheel
//! zoom produces one per notch. Issuing a query per step would put a DuckDB
//! round trip inside the pointer loop.
//!
//! The rule is **settle**: the extent moves the displayed scales on every step,
//! and exactly one re-query is issued once the gesture has ended. There is no
//! duration in it — the gesture-end test is the gesture's own shape (a released
//! button, a frame with no wheel delta, a keystroke that is over the moment it
//! is pressed), which is a fact about the input rather than a guess about how
//! fast a machine is.
//!
//! [`NavGesture`] is that rule and nothing else: steps go in, at most one
//! settled extent comes out per gesture, and the extents a gesture swept
//! through are recorded so "one query per gesture" is checkable rather than
//! asserted.
//!
//! # A categorical axis refuses, out loud
//!
//! Pan and zoom are arithmetic on a continuous domain. A band axis has
//! categories, not a range, so there is no such arithmetic to do — and the
//! filter a navigated extent would push (`"day" >= 1.5`) is not a question
//! anyone asked. Such an axis is REFUSED and the refusal is returned
//! ([`AxisRefusal`]) for the pane to say, rather than the gesture silently
//! doing nothing on that axis and reading as a broken control.

use brightfield_render::scale::{Scale, ScaleSet, ViewExtent};
use brightfield_render::channel::Channel;

/// Which axes a navigation gesture may move.
///
/// The lock is a property of the VIEW, not of the spec: it is the reader
/// saying "hold the other axis still while I look along this one", and it is
/// cycled from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisLock {
    /// Both axes move.
    #[default]
    Both,
    /// Only x moves; y is held.
    XOnly,
    /// Only y moves; x is held.
    YOnly,
}

impl AxisLock {
    /// The next lock in the cycle: both → x → y → both.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Both => Self::XOnly,
            Self::XOnly => Self::YOnly,
            Self::YOnly => Self::Both,
        }
    }

    /// Whether the x axis moves under this lock.
    #[must_use]
    pub fn moves_x(self) -> bool {
        matches!(self, Self::Both | Self::XOnly)
    }

    /// Whether the y axis moves under this lock.
    #[must_use]
    pub fn moves_y(self) -> bool {
        matches!(self, Self::Both | Self::YOnly)
    }

    /// How the status rail names this lock.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Both => "both axes",
            Self::XOnly => "x axis only",
            Self::YOnly => "y axis only",
        }
    }
}

/// Why an axis did not move under a navigation gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisRefusal {
    /// The axis is categorical — a band of categories, not a continuous range.
    /// There is no extent to pan or zoom, and no bound to push into SQL.
    Categorical,
    /// The plot draws nothing on this axis, so there is no scale to move.
    Missing,
}

impl AxisRefusal {
    /// The sentence a reader is shown.
    #[must_use]
    pub fn message(self, axis: &str) -> String {
        match self {
            Self::Categorical => format!(
                "the {axis} axis is categorical — pan and zoom need a continuous range"
            ),
            Self::Missing => format!("this plot has no {axis} axis to navigate"),
        }
    }
}

/// What one gesture produced: the extent to display, and every axis that
/// refused to take part.
#[derive(Debug, Clone, PartialEq)]
pub struct NavOutcome {
    /// The extent after the gesture. An axis that refused is `None` here,
    /// which is also what it was before — refusing changes nothing.
    pub extent: ViewExtent,
    /// Refusals, as `(axis name, why)`, in x-then-y order.
    pub refused: Vec<(&'static str, AxisRefusal)>,
}

impl NavOutcome {
    /// Whether every axis the lock allowed refused — the gesture did nothing at
    /// all, and the pane owes the reader a reason.
    #[must_use]
    pub fn is_inert(&self) -> bool {
        self.extent.x.is_none() && self.extent.y.is_none() && !self.refused.is_empty()
    }
}

/// One axis's continuous state as displayed: its data domain and its pixel
/// range. `None` for an axis that has no continuous domain.
fn continuous(scales: &ScaleSet, channel: Channel) -> Result<(f64, f64, f64, f64), AxisRefusal> {
    let Some(scale) = scales.get(channel) else {
        return Err(AxisRefusal::Missing);
    };
    match scale {
        Scale::Band { .. } => Err(AxisRefusal::Categorical),
        _ => {
            let (lo, hi) = (scale.domain_min(), scale.domain_max());
            let (Some(lo), Some(hi)) = (lo, hi) else {
                return Err(AxisRefusal::Categorical);
            };
            Ok((lo, hi, scale.range_start(), scale.range_end()))
        }
    }
}

/// Pan the displayed frame by a pixel delta.
///
/// **Read off the DISPLAYED scales, not off a remembered launch domain.** The
/// scales a plot was last drawn with already carry whatever extent is in force,
/// so a pan after a zoom composes correctly without this function knowing
/// anything about the gesture that came before it. Keeping a second copy of
/// "where the frame is" is how a pan and its picture drift apart.
///
/// `dx`/`dy` are the pointer's movement in logical pixels. The data under the
/// pointer stays under the pointer.
#[must_use]
pub fn pan(scales: &ScaleSet, lock: AxisLock, dx: f64, dy: f64) -> NavOutcome {
    let mut extent = ViewExtent::default();
    let mut refused = Vec::new();
    let mut axis = |channel: Channel, name: &'static str, delta: f64| -> Option<(f64, f64)> {
        match continuous(scales, channel) {
            Err(why) => {
                refused.push((name, why));
                None
            }
            Ok((lo, hi, r0, r1)) => {
                let span_px = r1 - r0;
                if span_px.abs() < f64::EPSILON {
                    refused.push((name, AxisRefusal::Missing));
                    return None;
                }
                let per_px = (hi - lo) / span_px;
                let shift = -delta * per_px;
                Some((lo + shift, hi + shift))
            }
        }
    };
    if lock.moves_x() {
        extent.x = axis(Channel::X, "x", dx);
    }
    if lock.moves_y() {
        extent.y = axis(Channel::Y, "y", dy);
    }
    NavOutcome { extent, refused }
}

/// Zoom the displayed frame about a cursor position.
///
/// `factor` greater than 1 zooms IN (the visible span shrinks). `cursor` is in
/// plot-local logical pixels; `None` zooms about the frame's centre, which is
/// what a keyboard zoom means.
#[must_use]
pub fn zoom(
    scales: &ScaleSet,
    lock: AxisLock,
    cursor: Option<(f64, f64)>,
    factor: f64,
) -> NavOutcome {
    let mut extent = ViewExtent::default();
    let mut refused = Vec::new();
    let mut axis = |channel: Channel, name: &'static str, at: Option<f64>| -> Option<(f64, f64)> {
        match continuous(scales, channel) {
            Err(why) => {
                refused.push((name, why));
                None
            }
            Ok((lo, hi, r0, r1)) => {
                if factor.abs() < f64::EPSILON || !factor.is_finite() {
                    refused.push((name, AxisRefusal::Missing));
                    return None;
                }
                let span_px = r1 - r0;
                let centre = match at {
                    Some(px) if span_px.abs() >= f64::EPSILON => {
                        lo + (px - r0) / span_px * (hi - lo)
                    }
                    _ => (lo + hi) / 2.0,
                };
                Some((
                    centre - (centre - lo) / factor,
                    centre + (hi - centre) / factor,
                ))
            }
        }
    };
    if lock.moves_x() {
        extent.x = axis(Channel::X, "x", cursor.map(|c| c.0));
    }
    if lock.moves_y() {
        extent.y = axis(Channel::Y, "y", cursor.map(|c| c.1));
    }
    NavOutcome { extent, refused }
}

/// How many settled extents [`NavGesture`] remembers. A session navigates for
/// as long as it likes; the record exists to be asserted against, not to grow.
const DISPATCH_LOG_CAP: usize = 64;

/// The settle rule: many steps in, at most one query out per gesture.
///
/// Every step updates what is DISPLAYED immediately (the caller reads
/// [`NavGesture::live`] each frame). Only [`NavGesture::settle`] arms a
/// dispatch, and only [`NavGesture::take_settled`] claims it — so a gesture
/// that is still moving has issued nothing, and a gesture that has ended has
/// issued exactly one thing.
#[derive(Debug, Default, Clone)]
pub struct NavGesture {
    /// The plot being navigated and the extent the gesture has reached.
    live: Option<(usize, ViewExtent)>,
    /// Whether the live extent is owed a query.
    owed: bool,
    /// Every extent actually handed out for dispatch, oldest first (bounded).
    dispatched: Vec<(usize, ViewExtent)>,
}

impl NavGesture {
    /// No gesture in progress.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything — the document this described has been replaced.
    pub fn clear(&mut self) {
        self.live = None;
        self.owed = false;
        self.dispatched.clear();
    }

    /// One step of a gesture on `plot`. Displayed at once; queried on settle.
    ///
    /// A step on a different plot replaces the live one outright: a gesture is
    /// on one plot at a time, and carrying the previous plot's pending extent
    /// forward would dispatch it against the wrong plot.
    pub fn moved(&mut self, plot: usize, extent: ViewExtent) {
        self.live = Some((plot, extent));
    }

    /// The gesture has ended: the live extent is now owed a query.
    ///
    /// A no-op when nothing is live, so a released button that never moved
    /// anything — or a settle called every idle frame — issues nothing.
    pub fn settle(&mut self) {
        if self.live.is_some() {
            self.owed = true;
        }
    }

    /// Claim the settled extent, once.
    ///
    /// Returns `Some` exactly once per settled gesture; the live extent stays
    /// (the frame is still there) but is no longer owed.
    pub fn take_settled(&mut self) -> Option<(usize, ViewExtent)> {
        if !self.owed {
            return None;
        }
        self.owed = false;
        let claimed = self.live.clone()?;
        if self.dispatched.len() == DISPATCH_LOG_CAP {
            self.dispatched.remove(0);
        }
        self.dispatched.push(claimed.clone());
        Some(claimed)
    }

    /// The extent the frame should be drawn at right now, if a gesture has
    /// touched it.
    #[must_use]
    pub fn live(&self) -> Option<(usize, &ViewExtent)> {
        self.live.as_ref().map(|(p, e)| (*p, e))
    }

    /// Whether a settled extent is waiting to be claimed.
    #[must_use]
    pub fn is_owed(&self) -> bool {
        self.owed
    }

    /// The extents actually handed out for dispatch, oldest first (bounded to
    /// the most recent [`DISPATCH_LOG_CAP`]).
    #[must_use]
    pub fn dispatched(&self) -> &[(usize, ViewExtent)] {
        &self.dispatched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_render::scale::Scale;

    fn linear_scales(x: (f64, f64), y: (f64, f64)) -> ScaleSet {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: x.0,
                domain_max: x.1,
                range_start: 0.0,
                range_end: 100.0,
            },
        );
        scales.insert(
            Channel::Y,
            Scale::Linear {
                domain_min: y.0,
                domain_max: y.1,
                // Screen y runs downward: the range is inverted, as every
                // rendered plot's is.
                range_start: 200.0,
                range_end: 0.0,
            },
        );
        scales
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_pan_keeps_the_data_under_the_pointer() {
        // 100 px spans 0..10, so one pixel is 0.1 data units. Dragging the
        // content 10 px to the right must move the frame 1.0 unit LEFT.
        let scales = linear_scales((0.0, 10.0), (0.0, 20.0));
        let out = pan(&scales, AxisLock::Both, 10.0, 0.0);
        let (lo, hi) = out.extent.x.expect("x panned");
        assert!(close(lo, -1.0) && close(hi, 9.0), "got {lo}..{hi}");
        assert!(out.refused.is_empty());
    }

    #[test]
    fn a_pan_composes_with_the_extent_already_displayed() {
        // The displayed scales ARE the current extent, so a second pan starts
        // from where the first left off with no state carried between them.
        let first = linear_scales((0.0, 10.0), (0.0, 20.0));
        let out = pan(&first, AxisLock::Both, 10.0, 0.0);
        let (lo, hi) = out.extent.x.expect("x panned");
        let second = linear_scales((lo, hi), (0.0, 20.0));
        let (lo, hi) = pan(&second, AxisLock::Both, 10.0, 0.0)
            .extent
            .x
            .expect("x panned again");
        assert!(close(lo, -2.0) && close(hi, 8.0), "got {lo}..{hi}");
    }

    #[test]
    fn a_zoom_about_the_cursor_holds_that_value_still() {
        let scales = linear_scales((0.0, 10.0), (0.0, 20.0));
        // Cursor at 25 px = data 2.5. Zooming in 2x must keep 2.5 where it is.
        let out = zoom(&scales, AxisLock::Both, Some((25.0, 0.0)), 2.0);
        let (lo, hi) = out.extent.x.expect("x zoomed");
        assert!(close(lo, 1.25) && close(hi, 6.25), "got {lo}..{hi}");
        assert!(close(lo + 0.25 * (hi - lo), 2.5), "the cursor value moved");
    }

    #[test]
    fn a_keyboard_zoom_with_no_cursor_works_from_the_centre() {
        let scales = linear_scales((0.0, 10.0), (0.0, 20.0));
        let (lo, hi) = zoom(&scales, AxisLock::Both, None, 2.0)
            .extent
            .x
            .expect("x zoomed");
        assert!(close(lo, 2.5) && close(hi, 7.5), "got {lo}..{hi}");
    }

    #[test]
    fn an_axis_lock_holds_the_other_axis_exactly_still() {
        let scales = linear_scales((0.0, 10.0), (0.0, 20.0));
        let out = pan(&scales, AxisLock::XOnly, 10.0, 10.0);
        assert!(out.extent.x.is_some());
        assert_eq!(out.extent.y, None, "y moved under an x-only lock");
        // And nothing is REFUSED — a locked axis is a choice, not a failure.
        assert!(out.refused.is_empty());
        assert_eq!(AxisLock::Both.cycle(), AxisLock::XOnly);
        assert_eq!(AxisLock::XOnly.cycle(), AxisLock::YOnly);
        assert_eq!(AxisLock::YOnly.cycle(), AxisLock::Both);
    }

    #[test]
    fn a_categorical_axis_refuses_and_says_which() {
        let mut scales = linear_scales((0.0, 10.0), (0.0, 20.0));
        scales.insert(
            Channel::Y,
            Scale::Band {
                categories: vec!["Mon".into(), "Tue".into()],
                range_start: 0.0,
                range_end: 100.0,
                padding: 0.1,
            },
        );
        let out = pan(&scales, AxisLock::Both, 10.0, 10.0);
        assert_eq!(out.extent.y, None);
        assert_eq!(out.refused, vec![("y", AxisRefusal::Categorical)]);
        assert!(out
            .refused
            .iter()
            .any(|(a, why)| why.message(a).contains("categorical")));

        // Locked to the categorical axis alone, the gesture is wholly inert —
        // which is the case that must never be silent.
        let out = pan(&scales, AxisLock::YOnly, 0.0, 10.0);
        assert!(out.is_inert(), "{out:?}");
    }

    #[test]
    fn an_absent_axis_refuses_as_missing_not_as_categorical() {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: 0.0,
                range_end: 10.0,
            },
        );
        let out = zoom(&scales, AxisLock::Both, None, 2.0);
        assert_eq!(out.refused, vec![("y", AxisRefusal::Missing)]);
    }

    // -----------------------------------------------------------------------
    // The settle rule
    // -----------------------------------------------------------------------

    fn ext(lo: f64, hi: f64) -> ViewExtent {
        ViewExtent {
            x: Some((lo, hi)),
            y: None,
        }
    }

    /// The whole point: a sustained gesture displays every step and queries
    /// once. The steps swept through are never dispatched.
    #[test]
    fn a_sustained_gesture_dispatches_once_at_its_end() {
        let mut nav = NavGesture::new();
        for i in 1..=8 {
            nav.moved(0, ext(0.0, 10.0 - f64::from(i)));
            assert_eq!(nav.take_settled(), None, "step {i} issued a query");
        }
        nav.settle();
        assert_eq!(nav.take_settled(), Some((0, ext(0.0, 2.0))));
        assert_eq!(nav.take_settled(), None, "settling dispatched twice");
        assert_eq!(
            nav.dispatched(),
            &[(0, ext(0.0, 2.0))],
            "an intermediate step reached the engine"
        );
    }

    /// A settle with nothing live issues nothing — a released button that
    /// never moved the frame is not a query.
    #[test]
    fn a_settle_without_a_step_issues_nothing() {
        let mut nav = NavGesture::new();
        nav.settle();
        assert_eq!(nav.take_settled(), None);
        assert!(nav.dispatched().is_empty());
    }

    /// Two gestures in a row are two queries, and the frame survives between
    /// them — the extent stays live after it has been claimed.
    #[test]
    fn a_second_gesture_dispatches_again_and_the_frame_persists() {
        let mut nav = NavGesture::new();
        nav.moved(0, ext(0.0, 5.0));
        nav.settle();
        assert!(nav.take_settled().is_some());
        assert_eq!(
            nav.live().map(|(p, e)| (p, e.clone())),
            Some((0, ext(0.0, 5.0))),
            "the claimed extent must stay displayed"
        );

        nav.moved(0, ext(1.0, 4.0));
        nav.settle();
        assert_eq!(nav.take_settled(), Some((0, ext(1.0, 4.0))));
        assert_eq!(nav.dispatched().len(), 2);
    }

    /// A gesture that moves to a second plot dispatches against THAT plot.
    #[test]
    fn a_gesture_on_another_plot_replaces_the_live_one() {
        let mut nav = NavGesture::new();
        nav.moved(0, ext(0.0, 5.0));
        nav.moved(1, ext(2.0, 3.0));
        nav.settle();
        assert_eq!(nav.take_settled(), Some((1, ext(2.0, 3.0))));
    }
}
