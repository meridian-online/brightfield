//! Where an interval slider's handle IS while it is being dragged, and which
//! of the values a drag produced ever reach DuckDB.
//!
//! Two problems, one small state machine, both of them about the gap between
//! the pointer and the last finished query.
//!
//! **The handle is UI-owned.** The controls rail used to re-seed every slider
//! from the composed dashboard each frame, so the rendered position was the
//! last *completed* query's value. Under a sustained drag that is always
//! behind the pointer, and when a re-query fails the rail silently snapped the
//! handle back to a value the user had moved away from — a picture disagreeing
//! with the hand on the control, with nothing on screen saying so. Here the
//! shown value is written by the pointer and by nothing else: it is seeded
//! from the composition once, and after that only [`IntervalDrags::note`]
//! moves it.
//!
//! **Coalescing, with no clock in it.** A drag emits a value per frame; a
//! re-query is slower than a frame. The rule is one dispatch outstanding at a
//! time and *the newest value wins*: a value noted while a dispatch is
//! outstanding replaces whatever was waiting, so the intermediate positions a
//! drag swept through are never executed — they are superseded before they are
//! dispatched. There is no timer and no debounce interval, because a threshold
//! is a guess about machine speed that is wrong on some machine; "one at a
//! time, newest wins" is true on every machine.
//!
//! A value the pointer released still lands: releasing does not clear the
//! pending slot, so the last value noted is dispatched as soon as the
//! outstanding one finishes, whether or not the pointer is still down.
//!
//! **This is deliberately UI-thread state.** The engine's [`QueryLoop`] moves
//! a whole `Coordinator` onto a worker thread by value; this shell reads the
//! same session synchronously each frame (the data grid borrows it), so
//! handing it away would break those reads. Coalescing does not need a thread
//! — it needs a pending slot and a rule. The rule is here; the dispatch stays
//! wherever the caller runs it.
//!
//! [`QueryLoop`]: brightfield_engine::coordinator::QueryLoop

use std::collections::HashMap;

/// One slider's live drag state.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Handle {
    /// Where the handle renders. The pointer's last value, always.
    shown: f64,
    /// The newest value not yet dispatched, if any.
    pending: Option<f64>,
    /// Whether a dispatch taken from this handle has not been reported
    /// finished yet.
    outstanding: bool,
}

/// The UI-owned drag state of every interval slider on a document, keyed by
/// [`IntervalControl::key`](crate::pipeline::IntervalControl::key).
#[derive(Debug, Default, Clone)]
pub struct IntervalDrags {
    handles: HashMap<String, Handle>,
    /// Every value that was actually handed out for dispatch, newest last —
    /// bounded, and read only by tests and by
    /// [`Self::dispatched`]. It is the record that makes "superseded values
    /// are never executed" checkable rather than asserted.
    dispatched: Vec<f64>,
}

/// How many dispatched values [`IntervalDrags`] remembers. A drag is thousands
/// of frames long over a session; the record exists to be asserted against,
/// not to grow.
const DISPATCH_LOG_CAP: usize = 64;

impl IntervalDrags {
    /// Empty state — no slider has been touched.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything. The document this state described has been replaced,
    /// so a handle position carried across would describe a slider that is no
    /// longer on screen.
    pub fn clear(&mut self) {
        self.handles.clear();
        self.dispatched.clear();
    }

    /// Where `key`'s handle should render, seeding from `boot` the first time
    /// this slider is seen.
    ///
    /// `boot` is the composition's value, and it is consulted **once**. After
    /// that the shown position is the pointer's, which is the whole point: a
    /// later composition carrying an older value cannot pull the handle out
    /// from under the hand holding it.
    ///
    /// The seed also arrives **owed a query**. A spec declares where its
    /// handle starts, but nothing has pushed that interval yet, so a first
    /// frame that only rendered the handle would draw a chart of everything
    /// under a control saying otherwise. Arming the seed makes the two agree
    /// from the first pump — and warms the pre-aggregate before a hand is on
    /// the slider, so the first drag step is already served from it.
    pub fn shown(&mut self, key: &str, boot: f64) -> f64 {
        self.handles
            .entry(key.to_string())
            .or_insert(Handle {
                shown: boot,
                pending: Some(boot),
                outstanding: false,
            })
            .shown
    }

    /// The pointer named `value` on `key`: it becomes what the handle shows,
    /// and it replaces any value still waiting to be dispatched.
    ///
    /// Replacing rather than queueing is the coalescing. A value noted and
    /// then noted over never reaches [`Self::take_dispatch`], so it is never
    /// executed.
    pub fn note(&mut self, key: &str, value: f64) {
        let handle = self.handles.entry(key.to_string()).or_insert(Handle {
            shown: value,
            pending: None,
            outstanding: false,
        });
        handle.shown = value;
        handle.pending = Some(value);
    }

    /// Claim `key`'s pending value for dispatch, or `None` when there is
    /// nothing waiting or a dispatch is already outstanding.
    ///
    /// Taking marks the handle outstanding; the caller must report back
    /// through [`Self::finish`] once its dispatch has returned, or this slider
    /// dispatches nothing further.
    pub fn take_dispatch(&mut self, key: &str) -> Option<f64> {
        let handle = self.handles.get_mut(key)?;
        if handle.outstanding {
            return None;
        }
        let value = handle.pending.take()?;
        handle.outstanding = true;
        if self.dispatched.len() == DISPATCH_LOG_CAP {
            self.dispatched.remove(0);
        }
        self.dispatched.push(value);
        Some(value)
    }

    /// Report that `key`'s outstanding dispatch has returned. Whether it
    /// succeeded is not this type's business: a failed re-query leaves the
    /// previous picture up, and the handle stays where the pointer put it
    /// either way.
    pub fn finish(&mut self, key: &str) {
        if let Some(handle) = self.handles.get_mut(key) {
            handle.outstanding = false;
        }
    }

    /// Whether `key` has a value waiting that no dispatch has claimed. The
    /// frame loop reads this to keep repainting while a drag has work left.
    #[must_use]
    pub fn is_pending(&self, key: &str) -> bool {
        self.handles.get(key).is_some_and(|h| h.pending.is_some())
    }

    /// Whether any slider has work waiting.
    #[must_use]
    pub fn any_pending(&self) -> bool {
        self.handles.values().any(|h| h.pending.is_some())
    }

    /// The values actually handed out for dispatch, oldest first (bounded to
    /// the most recent [`DISPATCH_LOG_CAP`]).
    #[must_use]
    pub fn dispatched(&self) -> &[f64] {
        &self.dispatched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: &str = "root/vconcat[1]/input[slider]";

    /// The spec's declared start is owed a query, so the first pump pushes it
    /// — the chart is filtered to what the handle says instead of showing
    /// everything under a control that claims otherwise.
    #[test]
    fn the_declared_start_is_dispatched_before_anything_is_dragged() {
        let mut drags = IntervalDrags::new();
        assert!((drags.shown(K, 60.0) - 60.0).abs() < f64::EPSILON);
        assert!(drags.is_pending(K), "the declared start is owed a query");
        assert_eq!(drags.take_dispatch(K), Some(60.0));
        drags.finish(K);
        assert!(!drags.any_pending(), "and only once");
    }

    /// The handle is the pointer's, not the query's: a value noted while a
    /// dispatch is outstanding still renders immediately, and the seed is
    /// consulted only before the first touch.
    #[test]
    fn the_rendered_handle_follows_the_pointer_not_the_last_query() {
        let mut drags = IntervalDrags::new();
        assert!(
            (drags.shown(K, 100.0) - 100.0).abs() < f64::EPSILON,
            "seeded"
        );

        drags.note(K, 40.0);
        let taken = drags.take_dispatch(K).expect("40 is dispatched");
        assert!((taken - 40.0).abs() < f64::EPSILON);

        // Mid-flight the pointer keeps moving. The handle keeps up even though
        // the composition still describes 40 — and a stale seed cannot pull it
        // back.
        drags.note(K, 55.0);
        assert!((drags.shown(K, 100.0) - 55.0).abs() < f64::EPSILON);
        assert!((drags.shown(K, 40.0) - 55.0).abs() < f64::EPSILON);
    }

    /// Coalescing: every value the drag swept between two dispatches is
    /// superseded, and only the newest is ever handed out.
    #[test]
    fn values_superseded_before_dispatch_are_never_dispatched() {
        let mut drags = IntervalDrags::new();
        drags.note(K, 10.0);
        assert_eq!(drags.take_dispatch(K), Some(10.0));

        // The whole sweep, while 10 is still outstanding.
        for v in [11.0, 12.0, 13.0, 14.0] {
            drags.note(K, v);
            assert_eq!(
                drags.take_dispatch(K),
                None,
                "a second dispatch was claimed while one was outstanding"
            );
        }
        drags.finish(K);
        assert_eq!(drags.take_dispatch(K), Some(14.0), "newest wins");
        drags.finish(K);
        assert_eq!(drags.take_dispatch(K), None, "nothing left waiting");

        assert_eq!(
            drags.dispatched(),
            &[10.0, 14.0],
            "11, 12 and 13 were superseded before dispatch and never executed"
        );
    }

    /// A drag that ends between dispatches still lands. Releasing does not
    /// clear the pending slot, so the released value goes out on the next
    /// claim — otherwise the chart would be left showing an intermediate
    /// position the user had already moved away from.
    #[test]
    fn a_value_released_between_dispatches_still_lands() {
        let mut drags = IntervalDrags::new();
        drags.note(K, 20.0);
        assert_eq!(drags.take_dispatch(K), Some(20.0));

        // The pointer moves once more and is released. No further `note`
        // arrives: the drag is over.
        drags.note(K, 31.0);
        drags.finish(K);
        assert!(drags.is_pending(K), "the released value is still owed");
        assert_eq!(drags.take_dispatch(K), Some(31.0));
        drags.finish(K);
        assert!(!drags.any_pending());
        assert!(
            (drags.shown(K, 0.0) - 31.0).abs() < f64::EPSILON,
            "and the handle rests where it was released"
        );
    }

    /// Sliders do not share a slot: one being outstanding does not hold the
    /// other up, and a cleared document forgets both.
    #[test]
    fn each_slider_holds_its_own_slot_and_a_new_document_forgets_them() {
        const OTHER: &str = "root/vconcat[2]/input[slider]";
        let mut drags = IntervalDrags::new();
        drags.note(K, 1.0);
        drags.note(OTHER, 2.0);
        assert_eq!(drags.take_dispatch(K), Some(1.0));
        assert_eq!(drags.take_dispatch(OTHER), Some(2.0));

        drags.clear();
        assert!(!drags.any_pending());
        assert!((drags.shown(K, 9.0) - 9.0).abs() < f64::EPSILON, "re-seeds");
        assert!(drags.dispatched().is_empty());
    }
}
