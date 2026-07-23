//! Animation transition state for mark-level interpolation.
//!
//! When data changes (new RecordBatch from re-query), marks animate from
//! their previous pixel positions to new positions. This module provides
//! the transition state machine — the actual interpolation happens in
//! each MarkRenderer's `render_interpolated()` method.

use std::time::{Duration, Instant};

/// Default data transition duration.
pub const DEFAULT_TRANSITION_DURATION: Duration = Duration::from_millis(300);

/// State of a mark transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionState {
    /// No transition in progress.
    Idle,
    /// Transition is actively animating.
    Running,
    /// Transition has completed (t >= 1.0).
    Complete,
}

/// A mark-level transition between previous and current positions.
///
/// Owns the previous pixel positions and tracks animation progress.
/// The `tick()` method returns the current interpolation factor `t`
/// (0.0 = prev, 1.0 = current) and updates the state.
pub struct Transition {
    /// Previous pixel positions per mark (x, y).
    pub prev_positions: Vec<(f64, f64)>,
    /// When the transition started.
    start: Instant,
    /// Total transition duration.
    duration: Duration,
}

impl Transition {
    /// Create a new transition with the given previous positions.
    pub fn new(prev_positions: Vec<(f64, f64)>, duration: Duration) -> Self {
        Self {
            prev_positions,
            start: Instant::now(),
            duration,
        }
    }

    /// Create a transition with a specific start time (for testing).
    pub fn new_at(prev_positions: Vec<(f64, f64)>, duration: Duration, start: Instant) -> Self {
        Self {
            prev_positions,
            start,
            duration,
        }
    }

    /// Compute the current interpolation factor and state.
    ///
    /// Returns `(t, state)` where `t` is clamped to [0.0, 1.0].
    /// Uses linear easing; a host's easing functions can be applied on top.
    pub fn tick(&self, now: Instant) -> (f64, TransitionState) {
        let elapsed = now.duration_since(self.start);
        if elapsed >= self.duration {
            (1.0, TransitionState::Complete)
        } else {
            let t = elapsed.as_secs_f64() / self.duration.as_secs_f64();
            (t, TransitionState::Running)
        }
    }

    /// Get the current state without computing t.
    pub fn state(&self, now: Instant) -> TransitionState {
        self.tick(now).1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_transition_is_running() {
        let t = Transition::new(vec![(0.0, 0.0)], DEFAULT_TRANSITION_DURATION);
        let (factor, state) = t.tick(Instant::now());
        assert_eq!(state, TransitionState::Running);
        assert!((0.0..1.0).contains(&factor));
    }

    #[test]
    fn past_duration_is_complete() {
        let start = Instant::now() - Duration::from_millis(500);
        let t = Transition::new_at(vec![(0.0, 0.0)], Duration::from_millis(300), start);
        let (factor, state) = t.tick(Instant::now());
        assert_eq!(state, TransitionState::Complete);
        assert!((factor - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn midpoint_returns_half() {
        let start = Instant::now();
        let duration = Duration::from_millis(200);
        let t = Transition::new_at(vec![(0.0, 0.0)], duration, start);

        // Tick at the midpoint
        let mid = start + Duration::from_millis(100);
        let (factor, state) = t.tick(mid);
        assert_eq!(state, TransitionState::Running);
        assert!(
            (factor - 0.5).abs() < 0.01,
            "midpoint t should be ~0.5, got {}",
            factor
        );
    }

    #[test]
    fn zero_elapsed_returns_zero() {
        let start = Instant::now();
        let t = Transition::new_at(vec![(10.0, 20.0)], Duration::from_millis(300), start);
        let (factor, state) = t.tick(start);
        assert_eq!(state, TransitionState::Running);
        assert!(factor < 0.01, "at start, t should be ~0, got {}", factor);
    }

    #[test]
    fn prev_positions_stored() {
        let positions = vec![(1.0, 2.0), (3.0, 4.0), (5.0, 6.0)];
        let t = Transition::new(positions.clone(), DEFAULT_TRANSITION_DURATION);
        assert_eq!(t.prev_positions, positions);
    }
}
