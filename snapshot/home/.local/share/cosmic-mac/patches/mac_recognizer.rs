// SPDX-License-Identifier: GPL-3.0-only
//
// cosmic-mac Stage A1: multi-finger swipe recognizer.
//
// PURE: no smithay, Wayland, Shell, D-Bus, rendering or compositor state. It is not wired into
// live input yet (that is step A3), so everything here is dead code in the running compositor.
#![allow(dead_code)]

//! A small state machine that turns raw touchpad swipe events into a locked axis plus a
//! progress/finish/cancel stream.
//!
//! ```text
//! Idle --Begin(3..4 fingers)--> Possible --dead zone + axis ratio--> Tracking(axis)
//! Tracking --End--> Finish   Tracking --End(cancelled)--> Cancel   Possible --End--> Cancel
//! ```
//!
//! # Units and signs
//! - All distances are in logical pixels, velocities in pixels per second, exactly as libinput
//!   reports gesture deltas. Positive x is to the right, positive y is down.
//! - `distance` is the signed movement **along the locked axis since the gesture began**
//!   (travel before the axis locked is not lost, so the content stays attached to the finger).
//! - The recognizer knows nothing about workspaces, natural scrolling or swipe length; the
//!   intent layer converts pixels to workspace units (see `workspace_transition`).
//!
//! # Not decided from the first event
//! The axis is only chosen once the accumulated movement has left the dead zone and one axis
//! dominates the other by `axis_ratio`. Until then nothing is emitted. After the axis is
//! locked it never changes and movement on the other axis is ignored.
//!
//! The configuration defaults are PROVISIONAL placeholders: they are to be tuned from real
//! touchpad recordings, not by assumption.

use std::{collections::VecDeque, time::Duration};

/// Velocity is measured over the most recent events within this window (same as `GestureState`).
const HISTORY_LIMIT: Duration = Duration::from_millis(150);
/// Same deceleration constant `GestureState::projected_end_pos` uses.
const DECELERATION_TOUCHPAD: f64 = 0.997;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecognizerConfig {
    /// Accumulated movement (px) needed before an axis may be chosen.
    pub dead_zone: f64,
    /// The dominant axis must be at least this many times the other axis to lock.
    pub axis_ratio: f64,
    /// Fewest fingers that start a recognition.
    pub min_fingers: u32,
    /// Most fingers that start a recognition.
    pub max_fingers: u32,
}

impl Default for RecognizerConfig {
    /// PROVISIONAL placeholder values, to be replaced from real touchpad recordings.
    fn default() -> Self {
        RecognizerConfig {
            dead_zone: 8.0,
            axis_ratio: 1.5,
            min_fingers: 3,
            max_fingers: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Raw input. `time` is a monotonic timestamp (libinput event time).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    Begin { fingers: u32, time: Duration },
    Update { dx: f64, dy: f64, time: Duration },
    End { cancelled: bool, time: Duration },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Output {
    /// Nothing to report (idle, still inside the dead zone, or axis not dominant yet).
    None,
    /// The axis was just locked. Emitted once, instead of the first `Progress`.
    Locked {
        axis: Axis,
        distance: f64,
        velocity: f64,
    },
    /// Movement along the locked axis.
    Progress {
        axis: Axis,
        distance: f64,
        velocity: f64,
    },
    /// Fingers lifted normally after the axis was locked.
    Finish {
        axis: Axis,
        distance: f64,
        velocity: f64,
        /// Where the gesture would end if it kept decelerating (px, along the axis).
        projected_end: f64,
    },
    /// Gesture cancelled, or ended without ever locking an axis. `distance` is the last
    /// along-axis distance (0 when the axis never locked).
    Cancel { distance: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// Fingers are down but the axis is not locked yet.
    Possible,
    Tracking(Axis),
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    time: Duration,
    dx: f64,
    dy: f64,
}

#[derive(Debug, Clone)]
pub struct Recognizer {
    config: RecognizerConfig,
    phase: Phase,
    /// Accumulated movement while `Possible`.
    sum: (f64, f64),
    /// Signed distance along the locked axis.
    distance: f64,
    history: VecDeque<Sample>,
}

impl Recognizer {
    pub fn new(config: RecognizerConfig) -> Self {
        Recognizer {
            config,
            phase: Phase::Idle,
            sum: (0.0, 0.0),
            distance: 0.0,
            history: VecDeque::new(),
        }
    }

    pub fn config(&self) -> &RecognizerConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: RecognizerConfig) {
        self.config = config;
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.sum = (0.0, 0.0);
        self.distance = 0.0;
        self.history.clear();
    }

    /// Feed one input event and get at most one output.
    ///
    /// A `Begin` while a gesture is already in progress abandons that gesture without an
    /// output and starts a new one (the ownership layer is responsible for cancelling
    /// properly; libinput always ends a gesture before starting the next).
    pub fn feed(&mut self, input: Input) -> Output {
        match input {
            Input::Begin { fingers, .. } => {
                self.reset();
                if fingers >= self.config.min_fingers && fingers <= self.config.max_fingers {
                    self.phase = Phase::Possible;
                }
                Output::None
            }
            Input::Update { dx, dy, time } => match self.phase {
                Phase::Idle => Output::None,
                Phase::Possible => {
                    self.sum.0 += dx;
                    self.sum.1 += dy;
                    self.push(time, dx, dy);
                    self.try_lock()
                }
                Phase::Tracking(axis) => {
                    self.distance += component(axis, dx, dy);
                    self.push(time, dx, dy);
                    Output::Progress {
                        axis,
                        distance: self.distance,
                        velocity: self.velocity(axis),
                    }
                }
            },
            Input::End { cancelled, time } => {
                // only movement within the last 150 ms before the fingers lift counts: resting
                // before lifting is not a flick
                self.history.retain(|s| time <= s.time + HISTORY_LIMIT);
                let out = match self.phase {
                    Phase::Idle => Output::None,
                    Phase::Possible => Output::Cancel { distance: 0.0 },
                    Phase::Tracking(axis) => {
                        if cancelled {
                            Output::Cancel {
                                distance: self.distance,
                            }
                        } else {
                            let velocity = self.velocity(axis);
                            Output::Finish {
                                axis,
                                distance: self.distance,
                                velocity,
                                projected_end: self.distance
                                    - velocity / (1000. * DECELERATION_TOUCHPAD.ln()),
                            }
                        }
                    }
                };
                self.reset();
                out
            }
        }
    }

    fn try_lock(&mut self) -> Output {
        let (sx, sy) = self.sum;
        let travelled = sx.hypot(sy);
        if travelled <= 0.0 || travelled < self.config.dead_zone {
            return Output::None;
        }
        let (axis, dominant, other) = if sx.abs() >= sy.abs() {
            (Axis::Horizontal, sx.abs(), sy.abs())
        } else {
            (Axis::Vertical, sy.abs(), sx.abs())
        };
        if dominant < self.config.axis_ratio * other {
            return Output::None;
        }
        self.phase = Phase::Tracking(axis);
        self.distance = component(axis, sx, sy);
        Output::Locked {
            axis,
            distance: self.distance,
            velocity: self.velocity(axis),
        }
    }

    fn push(&mut self, time: Duration, dx: f64, dy: f64) {
        // timestamps should increase monotonically; ignore any that go backwards
        if let Some(last) = self.history.back()
            && time < last.time
        {
            return;
        }
        self.history.push_back(Sample { time, dx, dy });
        while let (Some(first), Some(last)) = (self.history.front(), self.history.back()) {
            if last.time <= first.time + HISTORY_LIMIT {
                break;
            }
            self.history.pop_front();
        }
    }

    /// Velocity along `axis` in px/s over the recent history window.
    ///
    /// The first sample's delta is excluded: that movement happened *before* the window's first
    /// timestamp, so counting it would overestimate the speed by n/(n-1) (a 33% error with four
    /// samples). The stock `GestureState::velocity` includes it; it is left untouched in A1.
    fn velocity(&self, axis: Axis) -> f64 {
        let (Some(first), Some(last)) = (self.history.front(), self.history.back()) else {
            return 0.0;
        };
        let total_time = (last.time - first.time).as_secs_f64();
        if total_time == 0.0 {
            return 0.0;
        }
        let total: f64 = self
            .history
            .iter()
            .skip(1)
            .map(|s| component(axis, s.dx, s.dy))
            .sum();
        total / total_time
    }
}

fn component(axis: Axis, dx: f64, dy: f64) -> f64 {
    match axis {
        Axis::Horizontal => dx,
        Axis::Vertical => dy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RecognizerConfig {
        RecognizerConfig {
            dead_zone: 10.0,
            axis_ratio: 2.0,
            min_fingers: 3,
            max_fingers: 4,
        }
    }

    fn ms(t: u64) -> Duration {
        Duration::from_millis(t)
    }

    fn begin(r: &mut Recognizer, fingers: u32) -> Output {
        r.feed(Input::Begin {
            fingers,
            time: ms(0),
        })
    }

    fn upd(r: &mut Recognizer, dx: f64, dy: f64, t: u64) -> Output {
        r.feed(Input::Update {
            dx,
            dy,
            time: ms(t),
        })
    }

    fn end(r: &mut Recognizer, cancelled: bool, t: u64) -> Output {
        r.feed(Input::End {
            cancelled,
            time: ms(t),
        })
    }

    fn started(fingers: u32) -> Recognizer {
        let mut r = Recognizer::new(cfg());
        begin(&mut r, fingers);
        r
    }

    // ---- eligibility -------------------------------------------------------------------

    #[test]
    fn idle_ignores_updates_and_end() {
        let mut r = Recognizer::new(cfg());
        assert_eq!(upd(&mut r, 50.0, 0.0, 10), Output::None);
        assert_eq!(end(&mut r, false, 20), Output::None);
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[test]
    fn finger_count_outside_range_stays_idle() {
        for fingers in [1, 2, 5, 6] {
            let mut r = started(fingers);
            assert_eq!(r.phase(), Phase::Idle, "fingers = {fingers}");
            assert_eq!(upd(&mut r, 100.0, 0.0, 10), Output::None);
        }
    }

    #[test]
    fn three_and_four_fingers_both_start_recognition() {
        for fingers in [3, 4] {
            let r = started(fingers);
            assert_eq!(r.phase(), Phase::Possible, "fingers = {fingers}");
        }
    }

    // ---- dead zone ---------------------------------------------------------------------

    #[test]
    fn movement_inside_dead_zone_emits_nothing() {
        let mut r = started(3);
        assert_eq!(upd(&mut r, 3.0, 0.0, 10), Output::None);
        assert_eq!(upd(&mut r, 3.0, 1.0, 20), Output::None);
        assert_eq!(upd(&mut r, 2.0, 0.0, 30), Output::None);
        assert_eq!(r.phase(), Phase::Possible);
    }

    #[test]
    fn crossing_dead_zone_locks_axis_once() {
        let mut r = started(3);
        assert_eq!(upd(&mut r, 6.0, 0.0, 10), Output::None);
        match upd(&mut r, 6.0, 0.0, 20) {
            Output::Locked { axis, distance, .. } => {
                assert_eq!(axis, Axis::Horizontal);
                assert_eq!(distance, 12.0);
            }
            other => panic!("expected Locked, got {other:?}"),
        }
        assert_eq!(r.phase(), Phase::Tracking(Axis::Horizontal));
        // afterwards only Progress, never Locked again
        assert!(matches!(
            upd(&mut r, 5.0, 0.0, 30),
            Output::Progress {
                axis: Axis::Horizontal,
                ..
            }
        ));
    }

    #[test]
    fn single_large_first_event_still_respects_dead_zone_and_ratio() {
        // one big event is allowed to lock because it is clearly past the dead zone and dominant
        let mut r = started(3);
        assert!(matches!(upd(&mut r, 40.0, 2.0, 10), Output::Locked { .. }));
    }

    #[test]
    fn zero_movement_never_locks_even_with_zero_dead_zone() {
        let mut r = Recognizer::new(RecognizerConfig {
            dead_zone: 0.0,
            ..cfg()
        });
        begin(&mut r, 3);
        assert_eq!(upd(&mut r, 0.0, 0.0, 10), Output::None);
        assert_eq!(r.phase(), Phase::Possible);
    }

    // ---- axis locking and signs --------------------------------------------------------

    #[test]
    fn horizontal_lock_positive_is_rightwards() {
        let mut r = started(3);
        match upd(&mut r, 30.0, 0.0, 10) {
            Output::Locked { axis, distance, .. } => {
                assert_eq!(axis, Axis::Horizontal);
                assert!(distance > 0.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn horizontal_lock_negative_is_leftwards() {
        let mut r = started(3);
        match upd(&mut r, -30.0, 0.0, 10) {
            Output::Locked { axis, distance, .. } => {
                assert_eq!(axis, Axis::Horizontal);
                assert!(distance < 0.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn vertical_lock_up_is_negative_and_down_is_positive() {
        let mut up = started(3);
        match upd(&mut up, 0.0, -30.0, 10) {
            Output::Locked { axis, distance, .. } => {
                assert_eq!(axis, Axis::Vertical);
                assert!(distance < 0.0);
            }
            other => panic!("{other:?}"),
        }
        let mut down = started(3);
        match upd(&mut down, 0.0, 30.0, 10) {
            Output::Locked { axis, distance, .. } => {
                assert_eq!(axis, Axis::Vertical);
                assert!(distance > 0.0);
            }
            other => panic!("{other:?}"),
        }
    }

    // ---- diagonal ----------------------------------------------------------------------

    #[test]
    fn diagonal_start_waits_for_axis_ratio() {
        let mut r = started(3);
        // 45 degrees, well past the dead zone: ratio 1.0 < 2.0, so nothing locks
        assert_eq!(upd(&mut r, 20.0, 20.0, 10), Output::None);
        assert_eq!(upd(&mut r, 20.0, 20.0, 20), Output::None);
        assert_eq!(r.phase(), Phase::Possible);
    }

    #[test]
    fn diagonal_start_locks_once_one_axis_dominates() {
        let mut r = started(3);
        assert_eq!(upd(&mut r, 15.0, 15.0, 10), Output::None);
        // sum becomes (75, 15): ratio 5
        match upd(&mut r, 60.0, 0.0, 20) {
            Output::Locked { axis, distance, .. } => {
                assert_eq!(axis, Axis::Horizontal);
                assert_eq!(distance, 75.0, "no travel along the axis is lost before the lock");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn diagonal_that_never_dominates_ends_as_cancel() {
        let mut r = started(3);
        upd(&mut r, 20.0, 20.0, 10);
        upd(&mut r, -5.0, 5.0, 20);
        assert_eq!(end(&mut r, false, 30), Output::Cancel { distance: 0.0 });
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[test]
    fn tie_between_axes_does_not_lock_with_ratio_above_one() {
        let mut r = started(3);
        assert_eq!(upd(&mut r, 20.0, 20.0, 10), Output::None);
        assert_eq!(upd(&mut r, -20.0, -20.0, 20), Output::None); // back to zero travel
    }

    // ---- axis immutability and secondary axis ------------------------------------------

    #[test]
    fn axis_never_changes_after_lock() {
        let mut r = started(3);
        upd(&mut r, 40.0, 0.0, 10);
        assert_eq!(r.phase(), Phase::Tracking(Axis::Horizontal));
        // a large vertical movement afterwards must not re-lock to vertical
        upd(&mut r, 0.0, 300.0, 20);
        upd(&mut r, 0.0, -300.0, 30);
        assert_eq!(r.phase(), Phase::Tracking(Axis::Horizontal));
    }

    #[test]
    fn secondary_axis_movement_is_ignored_in_progress() {
        let mut r = started(3);
        upd(&mut r, 40.0, 0.0, 10);
        match upd(&mut r, 5.0, 200.0, 20) {
            Output::Progress { axis, distance, .. } => {
                assert_eq!(axis, Axis::Horizontal);
                assert_eq!(distance, 45.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn vertical_lock_ignores_horizontal_movement() {
        let mut r = started(3);
        upd(&mut r, 0.0, -40.0, 10);
        match upd(&mut r, 200.0, -5.0, 20) {
            Output::Progress { axis, distance, .. } => {
                assert_eq!(axis, Axis::Vertical);
                assert_eq!(distance, -45.0);
            }
            other => panic!("{other:?}"),
        }
    }

    // ---- cancellation ------------------------------------------------------------------

    #[test]
    fn cancelled_end_while_tracking_emits_cancel_with_distance() {
        let mut r = started(3);
        upd(&mut r, 40.0, 0.0, 10);
        upd(&mut r, 20.0, 0.0, 20);
        assert_eq!(end(&mut r, true, 30), Output::Cancel { distance: 60.0 });
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[test]
    fn cancelled_end_before_lock_emits_cancel() {
        let mut r = started(3);
        upd(&mut r, 3.0, 0.0, 10);
        assert_eq!(end(&mut r, true, 20), Output::Cancel { distance: 0.0 });
    }

    #[test]
    fn cancelled_end_is_never_a_finish() {
        let mut r = started(3);
        upd(&mut r, 100.0, 0.0, 10);
        upd(&mut r, 100.0, 0.0, 20);
        assert!(!matches!(end(&mut r, true, 30), Output::Finish { .. }));
    }

    #[test]
    fn recognizer_is_reusable_after_cancel() {
        let mut r = started(3);
        upd(&mut r, 40.0, 0.0, 10);
        end(&mut r, true, 20);
        begin(&mut r, 3);
        assert_eq!(r.phase(), Phase::Possible);
        match upd(&mut r, 0.0, -40.0, 30) {
            Output::Locked { axis, distance, .. } => {
                assert_eq!(axis, Axis::Vertical);
                assert_eq!(distance, -40.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn begin_while_active_abandons_previous_gesture() {
        let mut r = started(3);
        upd(&mut r, 40.0, 0.0, 10);
        assert_eq!(begin(&mut r, 3), Output::None);
        assert_eq!(r.phase(), Phase::Possible);
        // distance restarted
        match upd(&mut r, 30.0, 0.0, 20) {
            Output::Locked { distance, .. } => assert_eq!(distance, 30.0),
            other => panic!("{other:?}"),
        }
    }

    // ---- begin/end without meaningful movement -----------------------------------------

    #[test]
    fn begin_then_end_without_updates_is_cancel() {
        let mut r = started(3);
        assert_eq!(end(&mut r, false, 10), Output::Cancel { distance: 0.0 });
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[test]
    fn tiny_movement_then_normal_end_is_cancel_not_finish() {
        let mut r = started(4);
        upd(&mut r, 2.0, 1.0, 10);
        upd(&mut r, 1.0, 0.0, 20);
        assert_eq!(end(&mut r, false, 30), Output::Cancel { distance: 0.0 });
    }

    // ---- fast flick, slow long swipe, reversal -----------------------------------------

    #[test]
    fn fast_flick_finishes_with_high_velocity() {
        let mut r = started(3);
        // 60 px in 60 ms: 1000 px/s
        for i in 1..=6u64 {
            upd(&mut r, 10.0, 0.0, i * 10);
        }
        match end(&mut r, false, 70) {
            Output::Finish {
                axis,
                distance,
                velocity,
                projected_end,
            } => {
                assert_eq!(axis, Axis::Horizontal);
                assert_eq!(distance, 60.0);
                assert!(velocity > 900.0, "velocity = {velocity}");
                assert!(projected_end > distance);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fast_flick_left_has_negative_velocity() {
        let mut r = started(3);
        for i in 1..=6u64 {
            upd(&mut r, -10.0, 0.0, i * 10);
        }
        match end(&mut r, false, 70) {
            Output::Finish {
                distance,
                velocity,
                projected_end,
                ..
            } => {
                assert_eq!(distance, -60.0);
                assert!(velocity < -900.0);
                assert!(projected_end < distance);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn slow_long_swipe_finishes_with_large_distance_and_low_velocity() {
        let mut r = started(3);
        // 400 px over 4 s (100 events of 4 px every 40 ms): 100 px/s
        for i in 1..=100u64 {
            upd(&mut r, 4.0, 0.0, i * 40);
        }
        match end(&mut r, false, 4100) {
            Output::Finish {
                distance, velocity, ..
            } => {
                assert_eq!(distance, 400.0);
                assert!((velocity - 100.0).abs() < 15.0, "velocity = {velocity}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn resting_before_lifting_has_zero_velocity() {
        let mut r = started(3);
        for i in 1..=10u64 {
            upd(&mut r, 10.0, 0.0, i * 10);
        }
        // fingers rest for half a second, then lift
        match end(&mut r, false, 600) {
            Output::Finish { velocity, distance, .. } => {
                assert_eq!(velocity, 0.0);
                assert_eq!(distance, 100.0);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lifting_right_after_moving_keeps_the_velocity() {
        let mut r = started(3);
        for i in 1..=10u64 {
            upd(&mut r, 10.0, 0.0, i * 10);
        }
        match end(&mut r, false, 110) {
            Output::Finish { velocity, .. } => assert!((velocity - 1000.0).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn velocity_matches_constant_speed_exactly() {
        // 5 px every 10 ms is exactly 500 px/s however many samples are in the window
        for n in [2u64, 3, 5, 12, 40] {
            let mut r = started(3);
            for i in 1..=n {
                upd(&mut r, 5.0, 0.0, i * 10);
            }
            match end(&mut r, false, n * 10 + 5) {
                Output::Finish { velocity, .. } => {
                    assert!((velocity - 500.0).abs() < 1e-6, "n {n}: {velocity}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn velocity_uses_only_the_last_150_ms() {
        let mut r = started(3);
        // fast start, then nearly stopped for a long time
        for i in 1..=5u64 {
            upd(&mut r, 20.0, 0.0, i * 10);
        }
        for i in 0..40u64 {
            upd(&mut r, 0.1, 0.0, 100 + i * 10);
        }
        match end(&mut r, false, 600) {
            Output::Finish { velocity, .. } => assert!(velocity < 20.0, "velocity = {velocity}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reversal_reduces_distance_and_flips_velocity_sign() {
        let mut r = started(3);
        for i in 1..=5u64 {
            upd(&mut r, 20.0, 0.0, i * 10);
        }
        // fast reversal
        for i in 6..=12u64 {
            upd(&mut r, -20.0, 0.0, i * 10);
        }
        match end(&mut r, false, 130) {
            Output::Finish {
                distance, velocity, ..
            } => {
                assert!(distance < 100.0);
                assert!(velocity < 0.0, "velocity = {velocity}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn distance_is_cumulative_and_signed_through_reversal() {
        let mut r = started(3);
        upd(&mut r, 50.0, 0.0, 10);
        upd(&mut r, -80.0, 0.0, 20);
        match upd(&mut r, 0.0, 0.0, 30) {
            Output::Progress { distance, .. } => assert_eq!(distance, -30.0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn out_of_order_timestamps_are_ignored_in_history() {
        let mut r = started(3);
        upd(&mut r, 10.0, 0.0, 100);
        upd(&mut r, 10.0, 0.0, 110);
        // a late event with an earlier timestamp must not panic or corrupt the velocity
        upd(&mut r, 10.0, 0.0, 50);
        match end(&mut r, false, 120) {
            Output::Finish { distance, .. } => assert_eq!(distance, 30.0),
            other => panic!("{other:?}"),
        }
    }
}
