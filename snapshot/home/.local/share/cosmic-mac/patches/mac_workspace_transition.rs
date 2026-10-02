// SPDX-License-Identifier: GPL-3.0-only
//
// cosmic-mac Stage A1: retargetable workspace transition.
//
// PURE: no smithay, Wayland, Shell, D-Bus, rendering or compositor state. It is not wired into
// the shell yet (that is step A2), so everything here is dead code in the running compositor.
#![allow(dead_code)]

//! A workspace transition is one scalar **position in workspace-index units**:
//!
//! ```text
//! workspace 0 = 0.0     workspace 1 = 1.0     workspace 2 = 2.0
//! 1.5 = halfway between workspace 1 and workspace 2
//! ```
//!
//! # Coordinate, velocity and sign contract
//! - `position` and `velocity` are in workspace units and workspace units per second.
//!   **Positive means toward a higher workspace index.**
//! - The recognizer works in pixels. Crossing into this model is only a division by the swipe
//!   length (never by the output width):
//!   `workspace_delta = pixel_distance / swipe_length`,
//!   `workspace_velocity = pixel_velocity / swipe_length`
//!   (see [`pixels_to_workspaces`] and [`pixel_velocity_to_workspaces`]). The intent layer
//!   decides the sign of the pixel value *before* converting, e.g. with natural scrolling a
//!   finger moving left (negative x) means "toward a higher workspace", so it passes `-x`.
//!   The same sign carries through delta, velocity, position, spring velocity and target.
//!
//! # Logical state vs visual state
//! The transition only owns the **visual** state (`position`). The logical state, the active
//! workspace, belongs to the shell:
//! - while the user is dragging, `active` is unchanged and `position` follows the fingers;
//! - only a successful completion changes `active` (the caller does that, after [`decide`]);
//! - a cancellation never activates anything, it only animates back to the origin;
//! - when the transition is idle, `position == active as f64`.
//!
//! # Retargeting contract
//! Whenever an animation is interrupted (a new drag, a release in another direction, a
//! keyboard or protocol request) the new motion starts from the **sampled current position
//! and velocity**; it never snaps back to the logical active workspace first.
//!
//! Target selection ([`decide`]) is separate from animation ([`Transition::release`]).

use std::time::{Duration, Instant};

use crate::{
    backend::render::animations::spring::{Spring, SpringParams},
    input::gestures::RubberBand,
};

/// Same tolerance used to decide that a position is "on" a workspace.
const SNAP_EPSILON: f64 = 1e-6;

// ---------------------------------------------------------------------------------------------
// pixel <-> workspace units
// ---------------------------------------------------------------------------------------------

/// `workspace_delta = pixel_distance / swipe_length`. Not related to the output width.
pub fn pixels_to_workspaces(pixels: f64, swipe_length: f64) -> f64 {
    pixels / swipe_length.max(1e-9)
}

/// `workspace_velocity = pixel_velocity / swipe_length`. Not related to the output width.
pub fn pixel_velocity_to_workspaces(pixels_per_second: f64, swipe_length: f64) -> f64 {
    pixels_per_second / swipe_length.max(1e-9)
}

// ---------------------------------------------------------------------------------------------
// target selection
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecideParams {
    /// Release speed (workspaces per second) at or above which the direction of motion decides.
    pub velocity_threshold: f64,
    /// Distance (workspaces) from the origin that must be passed to move when slow.
    pub position_threshold: f64,
}

/// Which workspace a released swipe should settle on. Pure; contains all the product rules.
///
/// - `position`: visual position at release (workspace units; overshoot past the ends is
///   treated as the edge workspace).
/// - `velocity`: release velocity (workspace units per second, positive = higher index).
/// - `origin`: the active workspace the gesture started from.
/// - `last`: highest valid workspace index (`workspace_count - 1`).
///
/// Rules:
/// - **Fast** (`|velocity| >= velocity_threshold`): the direction of motion wins. Positive
///   velocity goes to the next workspace after the current position, negative to the one
///   before it. A fast reversal therefore cancels back toward the origin.
/// - **Slow**: stay on the origin unless the position moved more than `position_threshold`
///   away from it; then move toward that side (`ceil(|distance| - position_threshold)`
///   workspaces, which is one for a normal swipe).
/// - The result is always clamped to `0..=last`. `last == 0` always yields 0.
///
/// A cancelled gesture does not call this: it releases to `origin` directly.
pub fn decide(
    position: f64,
    velocity: f64,
    origin: usize,
    last: usize,
    params: DecideParams,
) -> usize {
    let p = position.clamp(0.0, last as f64);
    let target: i64 = if velocity.abs() >= params.velocity_threshold {
        if velocity > 0.0 {
            p.floor() as i64 + 1
        } else {
            p.ceil() as i64 - 1
        }
    } else {
        let d = p - origin as f64;
        if d.abs() > params.position_threshold {
            origin as i64 + d.signum() as i64 * (d.abs() - params.position_threshold).ceil() as i64
        } else {
            origin as i64
        }
    };
    target.clamp(0, last as i64) as usize
}

// ---------------------------------------------------------------------------------------------
// view: which workspaces to draw
// ---------------------------------------------------------------------------------------------

/// What the renderer (and later the pointer hit-testing) needs for one frame.
///
/// For `position = N + fraction` between two workspaces: `primary = N`,
/// `secondary = Some(N + 1)`, `fraction` in `(0, 1)`. On a workspace exactly, `secondary` is
/// `None` and `fraction`/`edge` are 0. Past the first workspace `primary = 0`,
/// `secondary = None` and `edge < 0`; past the last, `primary = last`, `secondary = None` and
/// `edge > 0`. `edge` is the overshoot in workspace units (a fraction of the output size).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub primary: usize,
    pub secondary: Option<usize>,
    pub fraction: f32,
    pub edge: f32,
}

/// The renderer and the pointer hit-testing draw "previous + current" with a direction and a
/// percentage (how far `previous` has moved out of its place). This maps a [`View`] onto that
/// existing model without changing it:
///
/// - `current` is the logical active workspace whenever it is one of the two drawn workspaces
///   (otherwise the higher one), `previous` is the other one;
/// - `forward` is `previous < current`, `percentage` the distance of the position from
///   `previous` (so it grows from 0 to 1 as `previous` leaves).
///
/// With no secondary workspace only `current` is drawn.
///
/// `direction` is the way the position is moving (see [`Transition::direction`]: positive toward
/// a higher index, negative toward a lower one, 0 when it is not moving). It matters when the
/// position is exactly on a workspace *while a switch is running*, which is the very first frame
/// of every switch: instead of reporting "only workspace N", the pair is drawn in the direction
/// of motion (`N` and its neighbour in that direction, at fraction 0 or 1). Rendering and
/// pointer hit-testing both go through this function, so they always agree on which workspace is
/// primary from the first frame of a transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairView {
    pub current: usize,
    pub previous: Option<PreviousView>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviousView {
    pub idx: usize,
    pub forward: bool,
    pub percentage: f32,
}

pub fn pair_view(view: &View, active: usize, last: usize, direction: i32) -> PairView {
    // exactly on a workspace while moving: use the pair in the direction of motion
    let (lo, hi, fraction) = match view.secondary {
        Some(hi) => (view.primary, hi, view.fraction),
        None if view.edge == 0.0 && direction > 0 && view.primary < last => {
            (view.primary, view.primary + 1, 0.0)
        }
        None if view.edge == 0.0 && direction < 0 && view.primary > 0 => {
            (view.primary - 1, view.primary, 1.0)
        }
        None => {
            return PairView {
                current: view.primary,
                previous: None,
            };
        }
    };
    if active == lo {
        // `lo` is the logical active workspace and `hi` is leaving: it moves out to the right
        PairView {
            current: lo,
            previous: Some(PreviousView {
                idx: hi,
                forward: false,
                percentage: 1.0 - fraction,
            }),
        }
    } else {
        // `hi` is the active workspace, or neither is (then the higher one is treated as current)
        PairView {
            current: hi,
            previous: Some(PreviousView {
                idx: lo,
                forward: true,
                percentage: fraction,
            }),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// curves
// ---------------------------------------------------------------------------------------------

/// EaseInOutCubic, the curve the shortcut animation uses today.
fn ease_in_out_cubic(s: f64) -> f64 {
    let s = s.clamp(0.0, 1.0);
    if s < 0.5 {
        4.0 * s * s * s
    } else {
        1.0 - 4.0 * (1.0 - s).powi(3)
    }
}

fn ease_in_out_cubic_derivative(s: f64) -> f64 {
    let s = s.clamp(0.0, 1.0);
    if s < 0.5 {
        12.0 * s * s
    } else {
        12.0 * (1.0 - s) * (1.0 - s)
    }
}

#[derive(Debug, Clone, Copy)]
enum Curve {
    /// Release of a swipe: the existing `Spring`, started from the sampled position/velocity.
    Spring(Spring),
    /// Fixed-duration ease. With `v0 == 0` this is exactly EaseInOutCubic from `from` to `to`.
    /// A non-zero initial velocity `v0` (workspaces/s) is honoured by adding
    /// `v0 * T * s * (1 - s)^2`, which starts with slope `v0`, vanishes at the end and keeps
    /// zero end velocity, so retargeting is continuous in position **and** velocity.
    Timed {
        from: f64,
        to: f64,
        v0: f64,
        duration: Duration,
    },
}

impl Curve {
    fn duration(&self) -> Duration {
        match self {
            Curve::Spring(spring) => spring.duration(),
            Curve::Timed { duration, .. } => *duration,
        }
    }

    fn position(&self, elapsed: Duration) -> f64 {
        match self {
            Curve::Spring(spring) => spring.value_at(elapsed),
            Curve::Timed {
                from,
                to,
                v0,
                duration,
            } => {
                let t_total = duration.as_secs_f64();
                if t_total <= 0.0 || elapsed >= *duration {
                    return *to;
                }
                let s = elapsed.as_secs_f64() / t_total;
                from + (to - from) * ease_in_out_cubic(s) + v0 * t_total * s * (1.0 - s).powi(2)
            }
        }
    }

    fn velocity(&self, elapsed: Duration) -> f64 {
        match self {
            Curve::Spring(spring) => spring.velocity_at(elapsed),
            Curve::Timed {
                from,
                to,
                v0,
                duration,
            } => {
                let t_total = duration.as_secs_f64();
                if t_total <= 0.0 || elapsed >= *duration {
                    return 0.0;
                }
                let s = elapsed.as_secs_f64() / t_total;
                (to - from) * ease_in_out_cubic_derivative(s) / t_total
                    + v0 * (1.0 - s) * (1.0 - 3.0 * s)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    /// At rest on a workspace. `position` is that workspace's index.
    Idle,
    /// Following the fingers. `anchor` is the position the gesture started from (the sampled
    /// visual position, which may be between workspaces if it interrupted an animation).
    Tracking { anchor: f64 },
    /// Playing a curve toward `target`.
    Animating {
        curve: Curve,
        start: Instant,
        target: usize,
    },
}

// ---------------------------------------------------------------------------------------------
// the transition
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Transition {
    /// Position while `Idle`/`Tracking` (workspace units).
    position: f64,
    /// Visual velocity while `Tracking` (workspace units per second).
    velocity: f64,
    mode: Mode,
}

impl Transition {
    /// Idle on workspace `active`.
    pub fn new(active: usize) -> Self {
        Transition {
            position: active as f64,
            velocity: 0.0,
            mode: Mode::Idle,
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.mode, Mode::Idle)
    }

    pub fn is_tracking(&self) -> bool {
        matches!(self.mode, Mode::Tracking { .. })
    }

    /// An animation is still playing at `now`.
    pub fn is_animating(&self, now: Instant) -> bool {
        match self.mode {
            Mode::Animating { curve, start, .. } => {
                now.saturating_duration_since(start) < curve.duration()
            }
            _ => false,
        }
    }

    /// Which way the position is moving at `now`: 1 toward a higher index, -1 toward a lower
    /// one, 0 when idle, tracking, arrived or exactly at its target. Based on the target of the
    /// running animation, so it is correct even on the first frame (velocity still zero).
    pub fn direction(&self, now: Instant) -> i32 {
        match self.mode {
            Mode::Animating { target, .. } => {
                let delta = target as f64 - self.position(now);
                if delta > SNAP_EPSILON {
                    1
                } else if delta < -SNAP_EPSILON {
                    -1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// The workspace an animation is heading to, if any.
    pub fn target(&self) -> Option<usize> {
        match self.mode {
            Mode::Animating { target, .. } => Some(target),
            _ => None,
        }
    }

    /// Visual position at `now` (workspace units).
    pub fn position(&self, now: Instant) -> f64 {
        match self.mode {
            Mode::Idle | Mode::Tracking { .. } => self.position,
            Mode::Animating {
                curve,
                start,
                target,
            } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= curve.duration() {
                    target as f64
                } else {
                    curve.position(elapsed)
                }
            }
        }
    }

    /// Visual velocity at `now` (workspace units per second, positive = higher index).
    pub fn velocity(&self, now: Instant) -> f64 {
        match self.mode {
            Mode::Idle => 0.0,
            Mode::Tracking { .. } => self.velocity,
            Mode::Animating { curve, start, .. } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed >= curve.duration() {
                    0.0
                } else {
                    curve.velocity(elapsed)
                }
            }
        }
    }

    /// Finish a completed animation: `Animating` becomes `Idle` exactly on its target.
    /// Returns true when that happened. Call this from the shell's per-frame refresh.
    pub fn refresh(&mut self, now: Instant) -> bool {
        if let Mode::Animating {
            curve,
            start,
            target,
        } = self.mode
            && now.saturating_duration_since(start) >= curve.duration()
        {
            self.position = target as f64;
            self.velocity = 0.0;
            self.mode = Mode::Idle;
            return true;
        }
        false
    }

    /// Jump to workspace `target` with no animation (for example while the overview is open).
    pub fn snap(&mut self, target: usize) {
        self.position = target as f64;
        self.velocity = 0.0;
        self.mode = Mode::Idle;
    }

    /// Start following the fingers. `active` is the logical active workspace.
    ///
    /// If an animation is running the drag adopts its current position and velocity (no jump,
    /// no snap back to `active`). If idle the position is `active` (restoring the invariant
    /// `position == active`).
    pub fn begin_tracking(&mut self, now: Instant, active: usize) {
        let (position, velocity) = match self.mode {
            Mode::Idle => (active as f64, 0.0),
            _ => (self.position(now), self.velocity(now)),
        };
        self.position = position;
        self.velocity = velocity;
        self.mode = Mode::Tracking { anchor: position };
    }

    /// Follow the fingers. `total_delta` is the total workspace-unit movement since the
    /// gesture began (positive = toward a higher index), `velocity` the current finger
    /// velocity in workspace units per second. The position is rubber-banded past the first
    /// and last workspace so the overshoot stays bounded. Ignored unless tracking.
    pub fn track(&mut self, total_delta: f64, velocity: f64, last: usize, band: &RubberBand) {
        if let Mode::Tracking { anchor } = self.mode {
            let raw = anchor + total_delta;
            let last = last as f64;
            self.position = band.clamp(0.0, last, raw);
            // visual velocity is the finger velocity scaled by the band's slope
            self.velocity = velocity * band.clamp_derivative(0.0, last, raw);
        }
    }

    /// Animate from the current visual state to workspace `target` with a spring. `velocity`
    /// is the starting velocity in workspace units per second (usually
    /// `self.velocity(now)`). Does not choose the target and never touches the logical active
    /// workspace; that is the caller's job (see [`decide`]).
    pub fn release(&mut self, now: Instant, target: usize, velocity: f64, params: SpringParams) {
        let from = self.position(now);
        let to = target as f64;
        if (from - to).abs() < SNAP_EPSILON && velocity.abs() < SNAP_EPSILON {
            self.snap(target);
            return;
        }
        self.mode = Mode::Animating {
            curve: Curve::Spring(Spring {
                from,
                to,
                initial_velocity: velocity,
                params,
            }),
            start: now,
            target,
        };
    }

    /// The fingers lifted normally: choose the target with [`decide`] and animate there from
    /// the current visual state. `origin` is the logical active workspace the gesture started
    /// from, `velocity` the release velocity (workspace units per second). Returns the target;
    /// the caller makes it the logical active workspace when it differs from `origin`.
    pub fn finish(
        &mut self,
        now: Instant,
        origin: usize,
        velocity: f64,
        last: usize,
        decide_params: DecideParams,
        spring: SpringParams,
    ) -> usize {
        let target = decide(self.position(now), velocity, origin, last, decide_params);
        let visual = self.velocity(now);
        self.release(now, target, visual, spring);
        target
    }

    /// The gesture was cancelled: animate back to the logical active workspace `origin`.
    /// Nothing is activated.
    pub fn cancel(&mut self, now: Instant, origin: usize, spring: SpringParams) {
        let visual = self.velocity(now);
        self.release(now, origin, visual, spring);
    }

    /// Animate from the current visual position **and velocity** to `target` over `duration`
    /// (keyboard shortcuts, protocol activation). From rest this is exactly the existing
    /// EaseInOutCubic.
    pub fn retarget_timed(&mut self, now: Instant, target: usize, duration: Duration) {
        let from = self.position(now);
        let v0 = self.velocity(now);
        let to = target as f64;
        if duration.is_zero() || ((from - to).abs() < SNAP_EPSILON && v0.abs() < SNAP_EPSILON) {
            self.snap(target);
            return;
        }
        self.mode = Mode::Animating {
            curve: Curve::Timed {
                from,
                to,
                v0,
                duration,
            },
            start: now,
            target,
        };
    }

    /// What to draw at `now` for a set with `workspace_count` workspaces (at least 1).
    pub fn view(&self, now: Instant, workspace_count: usize) -> View {
        let last = workspace_count.max(1) - 1;
        let p = self.position(now);
        if p < 0.0 {
            return View {
                primary: 0,
                secondary: None,
                fraction: 0.0,
                edge: p as f32,
            };
        }
        if p > last as f64 {
            return View {
                primary: last,
                secondary: None,
                fraction: 0.0,
                edge: (p - last as f64) as f32,
            };
        }
        let lo = p.floor();
        let frac = p - lo;
        if frac < SNAP_EPSILON {
            View {
                primary: lo as usize,
                secondary: None,
                fraction: 0.0,
                edge: 0.0,
            }
        } else if 1.0 - frac < SNAP_EPSILON {
            View {
                primary: (lo as usize + 1).min(last),
                secondary: None,
                fraction: 0.0,
                edge: 0.0,
            }
        } else {
            View {
                primary: lo as usize,
                secondary: Some(lo as usize + 1),
                fraction: frac as f32,
                edge: 0.0,
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Stage B2: keeping the transition on the same workspaces when the list changes
// ---------------------------------------------------------------------------------------------
//
// The transition lives in workspace-index units, but indices are only positions in the
// output's workspace list. When that list changes (pruning, removal, insertion, reordering) the
// same number can name a different workspace. Identity is the workspace handle, which the
// shell has and this module does not, so the shell passes a map from every old index to the
// new index of the same handle (see [`index_map`]).
//
// Model:
// - The **referenced span** is every workspace index the transition draws now or will still
//   draw: the integer neighbours of the current position, and while animating everything
//   between the current position and the target. Overshoot past either end belongs to that end
//   workspace. (The part of a curve already played is not referenced; the logical active
//   workspace is owned by the shell, which passes its new index.)
// - **Translation** (the normal case): if every workspace in the span survives and keeps the
//   same relative order and spacing (`new = old + d` for one `d`), the whole transition shifts
//   by `d`: position, tracking anchor, both ends of the running curve (springs and timed
//   curves are translation-invariant, so the motion continues exactly, only renumbered) and
//   the animation target. Nothing visible changes. `d == 0` (e.g. a change after the pair)
//   leaves the transition untouched.
// - **Fallback** (a referenced workspace was removed, or something was inserted or moved
//   *inside* the span): the old position can no longer be expressed in the new list without
//   naming a different workspace, so the transition is re-based on the logical active
//   workspace, `new_active`:
//   - idle or animating: snap to `new_active` (an animation always heads to the logical
//     active workspace, so this is where it was going to end anyway);
//   - tracking: keep tracking (the gesture is not taken away from the user), with the position
//     on `new_active` and the anchor shifted by the same amount, so further finger movement
//     continues relative to there. This is a visible jump, but only in this rare case and never
//     onto a stale index.

/// For each workspace in `before`, its index in `after` (`None` if it is gone). Workspaces are
/// compared by identity (the shell passes handles).
pub fn index_map<T: PartialEq>(before: &[T], after: &[T]) -> Vec<Option<usize>> {
    before
        .iter()
        .map(|b| after.iter().position(|a| a == b))
        .collect()
}

/// What [`Transition::remap`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemapOutcome {
    /// Nothing the transition references moved.
    Unchanged,
    /// Everything it references moved by this many places; the transition was shifted with it.
    Translated(isize),
    /// A referenced workspace was removed or the span was reordered; the transition was
    /// re-based on the logical active workspace.
    FellBackToActive,
}

impl Transition {
    /// The workspace indices (inclusive, within `0..workspace_count`) drawn now or still to be
    /// drawn by the running animation. `None` for an empty list.
    pub fn referenced_span(&self, now: Instant, workspace_count: usize) -> Option<(usize, usize)> {
        if workspace_count == 0 {
            return None;
        }
        let last = (workspace_count - 1) as f64;
        let p = self.position(now);
        let (lo, hi) = match self.mode {
            Mode::Animating { target, .. } => (p.min(target as f64), p.max(target as f64)),
            _ => (p, p),
        };
        // a position within SNAP_EPSILON of a workspace references only that workspace
        let lo = (lo.clamp(0.0, last) + SNAP_EPSILON).floor() as usize;
        let hi = (hi.clamp(0.0, last) - SNAP_EPSILON).ceil().max(0.0) as usize;
        Some((lo.min(hi), hi))
    }

    /// The workspace list changed. `map[i]` is the new index of the workspace that was at index
    /// `i` (see [`index_map`]), `new_count` the new number of workspaces and `new_active` the
    /// new index of the logical active workspace (the shell's, after the change). See the model
    /// above for what is kept and what happens when a referenced workspace is gone.
    pub fn remap(
        &mut self,
        now: Instant,
        map: &[Option<usize>],
        new_count: usize,
        new_active: usize,
    ) -> RemapOutcome {
        if new_count == 0 {
            // never left empty by the shell; still, never keep an index into nothing
            self.snap(0);
            return RemapOutcome::FellBackToActive;
        }
        let new_active = new_active.min(new_count - 1);
        let offset = |i: usize| -> Option<isize> {
            map.get(i)
                .copied()
                .flatten()
                .filter(|n| *n < new_count)
                .map(|n| n as isize - i as isize)
        };
        let translation = self.referenced_span(now, map.len()).and_then(|(lo, hi)| {
            let d = offset(lo)?;
            (lo..=hi).all(|i| offset(i) == Some(d)).then_some(d)
        });

        match translation {
            Some(0) => RemapOutcome::Unchanged,
            Some(d) => {
                self.translate(d, map);
                RemapOutcome::Translated(d)
            }
            None => {
                match self.mode {
                    Mode::Tracking { anchor } => {
                        let shift = new_active as f64 - self.position;
                        self.position = new_active as f64;
                        self.mode = Mode::Tracking {
                            anchor: anchor + shift,
                        };
                    }
                    _ => self.snap(new_active),
                }
                RemapOutcome::FellBackToActive
            }
        }
    }

    /// Shift everything by `d` workspaces. Only called when the referenced span (which always
    /// contains an animation's target) maps by exactly `d`.
    fn translate(&mut self, d: isize, map: &[Option<usize>]) {
        let df = d as f64;
        self.position += df;
        match &mut self.mode {
            Mode::Idle => {}
            Mode::Tracking { anchor } => *anchor += df,
            Mode::Animating { curve, target, .. } => {
                match curve {
                    Curve::Spring(spring) => {
                        spring.from += df;
                        spring.to += df;
                    }
                    Curve::Timed { from, to, .. } => {
                        *from += df;
                        *to += df;
                    }
                }
                // The target is in the span, so the map has it. A target already outside the
                // old list (stale) is shifted like everything else.
                *target = match map.get(*target).copied().flatten() {
                    Some(new) => new,
                    None => (*target as isize + d).max(0) as usize,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn params() -> SpringParams {
        // critically damped, a typical release spring (placeholder values for the tests only)
        SpringParams::new(1.0, 800.0, 0.0001)
    }

    fn decide_params() -> DecideParams {
        DecideParams {
            velocity_threshold: 0.5,
            position_threshold: 0.5,
        }
    }

    fn band() -> RubberBand {
        RubberBand {
            stiffness: 0.5,
            limit: 0.25,
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    fn after(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// A minimal stand-in for the shell: owns the logical `active` and applies the rules the
    /// real shell will apply, so the active-vs-visual invariants can be tested.
    struct Sim {
        active: usize,
        count: usize,
        t: Transition,
        total: f64, // total workspace delta since the gesture began
    }

    impl Sim {
        fn new(active: usize, count: usize) -> Self {
            Sim {
                active,
                count,
                t: Transition::new(active),
                total: 0.0,
            }
        }

        fn last(&self) -> usize {
            self.count - 1
        }

        fn begin(&mut self, now: Instant) {
            self.total = 0.0;
            self.t.begin_tracking(now, self.active);
        }

        fn drag(&mut self, delta: f64, velocity: f64) {
            self.total = delta;
            let last = self.last();
            self.t.track(delta, velocity, last, &band());
        }

        fn finish(&mut self, now: Instant, velocity: f64) {
            let pos = self.t.position(now);
            let target = decide(pos, velocity, self.active, self.last(), decide_params());
            if target != self.active {
                self.active = target; // only a successful completion changes `active`
            }
            let v = self.t.velocity(now);
            self.t.release(now, target, v, params());
        }

        fn cancel(&mut self, now: Instant) {
            let v = self.t.velocity(now);
            let origin = self.active;
            self.t.release(now, origin, v, params()); // never activates
        }
    }

    // =========================================================================================
    // units and sign convention
    // =========================================================================================

    #[test]
    fn pixels_convert_by_swipe_length_not_output_width() {
        assert!(near(pixels_to_workspaces(300.0, 600.0), 0.5, EPS));
        assert!(near(pixels_to_workspaces(-300.0, 600.0), -0.5, EPS));
        assert!(near(pixel_velocity_to_workspaces(1200.0, 600.0), 2.0, EPS));
        assert!(near(pixel_velocity_to_workspaces(-1200.0, 600.0), -2.0, EPS));
        // changing nothing but the swipe length changes the result
        assert!(near(pixels_to_workspaces(300.0, 300.0), 1.0, EPS));
    }

    #[test]
    fn zero_swipe_length_does_not_divide_by_zero() {
        assert!(pixels_to_workspaces(10.0, 0.0).is_finite());
        assert!(pixel_velocity_to_workspaces(10.0, 0.0).is_finite());
    }

    // =========================================================================================
    // decision logic
    // =========================================================================================

    #[test]
    fn slow_release_below_position_threshold_stays_on_origin_both_directions() {
        let p = decide_params();
        assert_eq!(decide(1.3, 0.0, 1, 3, p), 1);
        assert_eq!(decide(0.7, 0.0, 1, 3, p), 1);
        assert_eq!(decide(1.5, 0.0, 1, 3, p), 1); // exactly at the threshold does not move
    }

    #[test]
    fn slow_release_past_position_threshold_moves_toward_that_side() {
        let p = decide_params();
        assert_eq!(decide(1.6, 0.0, 1, 3, p), 2);
        assert_eq!(decide(0.4, 0.0, 1, 3, p), 0);
    }

    #[test]
    fn slow_release_far_past_threshold_can_move_more_than_one() {
        let p = decide_params();
        assert_eq!(decide(2.7, 0.0, 0, 3, p), 3); // d = 2.7, ceil(2.2) = 3
        assert_eq!(decide(0.2, 0.0, 3, 3, p), 0); // d = -2.8, ceil(2.3) = 3
    }

    #[test]
    fn fast_positive_velocity_commits_to_next_regardless_of_small_distance() {
        let p = decide_params();
        assert_eq!(decide(0.1, 2.0, 0, 3, p), 1);
        assert_eq!(decide(0.9, 2.0, 0, 3, p), 1);
    }

    #[test]
    fn fast_negative_velocity_commits_to_previous() {
        let p = decide_params();
        assert_eq!(decide(0.9, -2.0, 1, 3, p), 0);
        assert_eq!(decide(1.1, -2.0, 1, 3, p), 1);
    }

    #[test]
    fn fast_reversal_returns_toward_origin() {
        let p = decide_params();
        // dragged from 0 to 0.8, then flicked back
        assert_eq!(decide(0.8, -2.0, 0, 3, p), 0);
        // dragged from 2 to 1.2, then flicked back toward 2
        assert_eq!(decide(1.2, 2.0, 2, 3, p), 2);
    }

    #[test]
    fn velocity_threshold_is_inclusive() {
        let p = decide_params();
        // at the threshold the velocity rule applies even though the position says "stay"
        assert_eq!(decide(0.1, 0.5, 0, 3, p), 1);
        // just below it the position rule applies
        assert_eq!(decide(0.1, 0.4999, 0, 3, p), 0);
    }

    #[test]
    fn velocity_overrides_position_in_both_signs() {
        let p = decide_params();
        // past the position threshold but flicked back
        assert_eq!(decide(0.8, -1.0, 0, 3, p), 0);
        // before the position threshold but flicked forward
        assert_eq!(decide(0.2, 1.0, 0, 3, p), 1);
    }

    #[test]
    fn decide_with_one_workspace_is_always_zero() {
        let p = decide_params();
        for pos in [-1.0, -0.2, 0.0, 0.3, 5.0] {
            for vel in [-5.0, -0.1, 0.0, 0.1, 5.0] {
                assert_eq!(decide(pos, vel, 0, 0, p), 0, "pos {pos} vel {vel}");
            }
        }
    }

    #[test]
    fn decide_with_two_workspaces_picks_zero_or_one() {
        let p = decide_params();
        assert_eq!(decide(0.7, 0.0, 0, 1, p), 1);
        assert_eq!(decide(0.3, 0.0, 0, 1, p), 0);
        assert_eq!(decide(0.3, 3.0, 0, 1, p), 1);
        assert_eq!(decide(0.7, -3.0, 1, 1, p), 0);
        // never beyond the last of two
        assert_eq!(decide(1.0, 9.0, 1, 1, p), 1);
        // never below the first
        assert_eq!(decide(0.0, -9.0, 0, 1, p), 0);
    }

    #[test]
    fn decide_clamps_overshoot_and_out_of_range_results() {
        let p = decide_params();
        assert_eq!(decide(-0.2, -5.0, 0, 3, p), 0);
        assert_eq!(decide(3.2, 5.0, 3, 3, p), 3);
        assert_eq!(decide(-0.2, 0.0, 0, 3, p), 0);
        assert_eq!(decide(3.2, 0.0, 3, 3, p), 3);
        assert_eq!(decide(1.0, 99.0, 3, 3, p), 2); // from origin 3, dragged back to 1.0, then flicked forward
    }

    // =========================================================================================
    // spring velocity
    // =========================================================================================

    fn numeric_velocity(spring: &Spring, t: f64) -> f64 {
        let h = 1e-6;
        let a = spring.value_at(Duration::from_secs_f64((t - h).max(0.0)));
        let b = spring.value_at(Duration::from_secs_f64(t + h));
        let denom = if t - h < 0.0 { t + h } else { 2.0 * h };
        (b - a) / denom
    }

    fn spring_with(from: f64, to: f64, v0: f64, damping_ratio: f64, stiffness: f64) -> Spring {
        Spring {
            from,
            to,
            initial_velocity: v0,
            params: SpringParams::new(damping_ratio, stiffness, 0.0001),
        }
    }

    #[test]
    fn velocity_at_zero_equals_initial_velocity() {
        for (from, to, v0) in [(0.0, 1.0, 0.0), (0.0, 1.0, 2.5), (1.0, 0.0, -3.0), (0.4, 2.0, 1.0)] {
            for damping in [0.6, 1.0, 1.7] {
                let s = spring_with(from, to, v0, damping, 800.0);
                assert!(
                    near(s.velocity_at(Duration::ZERO), v0, 1e-9),
                    "from {from} to {to} v0 {v0} damping {damping}"
                );
            }
        }
    }

    #[test]
    fn velocity_at_matches_numeric_derivative_for_all_damping_regimes() {
        for damping in [0.5, 0.9, 1.0, 1.4, 3.0] {
            // underdamped, near/at critical, overdamped
            let s = spring_with(0.0, 1.0, 1.5, damping, 600.0);
            for ms in [1u64, 5, 20, 50, 100, 200, 400] {
                let t = ms as f64 / 1000.0;
                let analytic = s.velocity_at(Duration::from_secs_f64(t));
                let numeric = numeric_velocity(&s, t);
                assert!(
                    near(analytic, numeric, 1e-3 * (1.0 + numeric.abs())),
                    "damping {damping} t {t}: analytic {analytic} numeric {numeric}"
                );
            }
        }
    }

    #[test]
    fn velocity_at_positive_and_negative_direction() {
        // moving up toward 1: velocity positive early
        let up = spring_with(0.0, 1.0, 0.0, 1.0, 800.0);
        assert!(up.velocity_at(Duration::from_millis(30)) > 0.0);
        // moving down toward 0: velocity negative early
        let down = spring_with(1.0, 0.0, 0.0, 1.0, 800.0);
        assert!(down.velocity_at(Duration::from_millis(30)) < 0.0);
    }

    #[test]
    fn velocity_at_with_from_greater_than_to() {
        let s = spring_with(3.0, 1.0, -1.0, 0.8, 500.0);
        for ms in [2u64, 25, 90, 250] {
            let t = ms as f64 / 1000.0;
            let analytic = s.velocity_at(Duration::from_secs_f64(t));
            let numeric = numeric_velocity(&s, t);
            assert!(near(analytic, numeric, 1e-3 * (1.0 + numeric.abs())), "t {t}");
        }
    }

    #[test]
    fn velocity_at_with_values_outside_zero_to_one() {
        for (from, to) in [(-0.4, 1.0), (1.6, 0.0), (-2.0, -1.0), (5.0, 7.5)] {
            let s = spring_with(from, to, 0.7, 1.0, 700.0);
            for ms in [3u64, 40, 120, 300] {
                let t = ms as f64 / 1000.0;
                let analytic = s.velocity_at(Duration::from_secs_f64(t));
                let numeric = numeric_velocity(&s, t);
                assert!(
                    near(analytic, numeric, 1e-3 * (1.0 + numeric.abs())),
                    "from {from} to {to} t {t}"
                );
            }
        }
    }

    #[test]
    fn velocity_at_decays_to_zero() {
        let s = spring_with(0.0, 1.0, 2.0, 1.0, 800.0);
        assert!(s.velocity_at(Duration::from_secs(3)).abs() < 1e-6);
    }

    // =========================================================================================
    // curves
    // =========================================================================================

    #[test]
    fn ease_in_out_cubic_matches_known_values() {
        assert!(near(ease_in_out_cubic(0.0), 0.0, EPS));
        assert!(near(ease_in_out_cubic(0.25), 0.0625, EPS));
        assert!(near(ease_in_out_cubic(0.5), 0.5, EPS));
        assert!(near(ease_in_out_cubic(0.75), 0.9375, EPS));
        assert!(near(ease_in_out_cubic(1.0), 1.0, EPS));
    }

    #[test]
    fn timed_curve_from_rest_is_exactly_ease_in_out_cubic() {
        let now = t0();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 2, Duration::from_millis(200));
        for ms in [0u64, 20, 50, 100, 150, 199] {
            let s = ms as f64 / 200.0;
            let expected = 2.0 * ease_in_out_cubic(s);
            assert!(near(t.position(after(now, ms)), expected, 1e-9), "ms {ms}");
        }
        assert!(near(t.position(after(now, 200)), 2.0, EPS));
        assert!(near(t.position(after(now, 5000)), 2.0, EPS));
    }

    #[test]
    fn timed_curve_has_zero_velocity_at_both_ends_from_rest() {
        let now = t0();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 1, Duration::from_millis(200));
        assert!(near(t.velocity(now), 0.0, 1e-9));
        assert!(near(t.velocity(after(now, 200)), 0.0, 1e-9));
        // fastest in the middle, heading to a higher index
        assert!(t.velocity(after(now, 100)) > 0.0);
    }

    #[test]
    fn timed_curve_velocity_matches_numeric_derivative_with_initial_velocity() {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        t.track(0.4, 3.0, 4, &band());
        t.retarget_timed(now, 3, Duration::from_millis(300));
        for ms in [1u64, 10, 60, 150, 250, 290] {
            let h = 0.0005;
            let a = t.position(now + Duration::from_secs_f64((ms as f64 / 1000.0 - h).max(0.0)));
            let b = t.position(now + Duration::from_secs_f64(ms as f64 / 1000.0 + h));
            let numeric = (b - a) / (2.0 * h);
            let analytic = t.velocity(after(now, ms));
            assert!(near(analytic, numeric, 5e-3 * (1.0 + numeric.abs())), "ms {ms}");
        }
    }

    #[test]
    fn timed_curve_with_zero_duration_snaps() {
        let now = t0();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 2, Duration::ZERO);
        assert!(t.is_idle());
        assert_eq!(t.position(now), 2.0);
    }

    // =========================================================================================
    // idle / active invariant
    // =========================================================================================

    #[test]
    fn idle_position_equals_active() {
        let now = t0();
        for active in 0..5usize {
            let t = Transition::new(active);
            assert!(t.is_idle());
            assert!(near(t.position(now), active as f64, EPS));
            assert_eq!(t.velocity(now), 0.0);
        }
    }

    #[test]
    fn invariant_holds_after_every_kind_of_completion() {
        let now = t0();
        // timed
        let mut t = Transition::new(1);
        t.retarget_timed(now, 3, Duration::from_millis(200));
        assert!(t.refresh(after(now, 250)));
        assert!(t.is_idle());
        assert!(near(t.position(after(now, 250)), 3.0, EPS));
        // spring
        let mut t = Transition::new(2);
        t.begin_tracking(now, 2);
        t.track(-0.6, -1.0, 4, &band());
        t.release(now, 1, -1.0, params());
        assert!(t.refresh(after(now, 5000)));
        assert!(near(t.position(after(now, 5000)), 1.0, EPS));
        // snap
        let mut t = Transition::new(0);
        t.snap(4);
        assert!(near(t.position(now), 4.0, EPS));
    }

    #[test]
    fn refresh_does_nothing_while_playing_or_idle() {
        let now = t0();
        let mut t = Transition::new(0);
        assert!(!t.refresh(now));
        t.retarget_timed(now, 1, Duration::from_millis(200));
        assert!(!t.refresh(after(now, 100)));
        assert!(t.is_animating(after(now, 100)));
        assert!(!t.is_animating(after(now, 200)));
    }

    #[test]
    fn begin_tracking_when_idle_restores_position_to_active() {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 2); // active says 2, so tracking starts from 2.0
        assert!(near(t.position(now), 2.0, EPS));
    }

    // =========================================================================================
    // active vs visual during a gesture
    // =========================================================================================

    #[test]
    fn active_is_unchanged_while_dragging_and_position_follows() {
        let now = t0();
        let mut sim = Sim::new(1, 4);
        sim.begin(now);
        sim.drag(0.3, 0.5);
        assert_eq!(sim.active, 1);
        assert!(near(sim.t.position(now), 1.3, EPS));
        sim.drag(-0.4, -0.5);
        assert_eq!(sim.active, 1);
        assert!(near(sim.t.position(now), 0.6, EPS));
    }

    #[test]
    fn successful_completion_changes_active_and_converges_to_it() {
        let now = t0();
        let mut sim = Sim::new(1, 4);
        sim.begin(now);
        sim.drag(0.8, 1.0);
        sim.finish(now, 1.0);
        assert_eq!(sim.active, 2);
        let later = after(now, 5000);
        assert!(sim.t.refresh(later));
        assert!(near(sim.t.position(later), 2.0, EPS));
    }

    #[test]
    fn cancel_never_activates_and_returns_to_origin() {
        let now = t0();
        let mut sim = Sim::new(1, 4);
        sim.begin(now);
        sim.drag(0.9, 2.0); // looked like it would commit
        sim.cancel(now);
        assert_eq!(sim.active, 1, "cancel must never change the active workspace");
        let later = after(now, 5000);
        sim.t.refresh(later);
        assert!(near(sim.t.position(later), 1.0, EPS));
    }

    #[test]
    fn cancel_in_the_negative_direction_never_activates() {
        let now = t0();
        let mut sim = Sim::new(2, 4);
        sim.begin(now);
        sim.drag(-0.9, -2.0);
        sim.cancel(now);
        assert_eq!(sim.active, 2);
        let later = after(now, 5000);
        sim.t.refresh(later);
        assert!(near(sim.t.position(later), 2.0, EPS));
    }

    #[test]
    fn finish_with_tiny_movement_keeps_active_and_returns() {
        let now = t0();
        let mut sim = Sim::new(1, 4);
        sim.begin(now);
        sim.drag(0.1, 0.05);
        sim.finish(now, 0.05);
        assert_eq!(sim.active, 1);
        let later = after(now, 5000);
        sim.t.refresh(later);
        assert!(near(sim.t.position(later), 1.0, EPS));
    }

    #[test]
    fn both_directions_commit_with_matching_signs() {
        let now = t0();
        let mut forward = Sim::new(1, 4);
        forward.begin(now);
        forward.drag(0.7, 1.0);
        forward.finish(now, 1.0);
        assert_eq!(forward.active, 2);

        let mut backward = Sim::new(1, 4);
        backward.begin(now);
        backward.drag(-0.7, -1.0);
        backward.finish(now, -1.0);
        assert_eq!(backward.active, 0);
    }

    // =========================================================================================
    // continuity and retargeting
    // =========================================================================================

    #[test]
    fn begin_tracking_adopts_current_position_and_velocity_not_active() {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        t.track(0.6, 1.5, 3, &band());
        t.release(now, 1, 1.5, params());
        let mid = after(now, 40);
        let pos = t.position(mid);
        let vel = t.velocity(mid);
        assert!(pos > 0.6 && pos < 1.0, "pos {pos}");
        // a new gesture starts while the spring is still running, logical active is now 1
        t.begin_tracking(mid, 1);
        assert!(near(t.position(mid), pos, EPS), "must not snap to the logical active workspace");
        assert!(near(t.velocity(mid), vel, EPS), "velocity must carry over too");
        // tracking with zero delta stays exactly there
        t.track(0.0, vel, 3, &band());
        assert!(near(t.position(mid), pos, EPS));
    }

    #[test]
    fn a_to_b_then_c_without_visual_jumps() {
        let now = t0();
        let mut sim = Sim::new(0, 4);
        // gesture 1: A (0) toward B (1)
        sim.begin(now);
        sim.drag(0.45, 1.0);
        let p_before = sim.t.position(now);
        sim.finish(now, 1.2);
        assert_eq!(sim.active, 1);
        assert!(near(sim.t.position(now), p_before, 1e-9), "release must start where the finger was");

        // 60 ms later, still on its way, the user swipes toward C (2)
        let mid = after(now, 60);
        let p_mid = sim.t.position(mid);
        let v_mid = sim.t.velocity(mid);
        sim.begin(mid);
        assert!(near(sim.t.position(mid), p_mid, 1e-9), "gesture 2 must begin at the visual position");
        assert!(near(sim.t.velocity(mid), v_mid, 1e-9));
        sim.drag(0.3, 1.0);
        let p_drag = sim.t.position(mid);
        assert!(near(p_drag, p_mid + 0.3, 1e-9));
        sim.finish(mid, 1.2);
        assert_eq!(sim.active, 2);
        assert!(near(sim.t.position(mid), p_drag, 1e-9), "release must start at the dragged position");

        // no sample-to-sample jump while it settles
        let mut last = sim.t.position(mid);
        for ms in 1..600u64 {
            let p = sim.t.position(mid + Duration::from_millis(ms));
            assert!((p - last).abs() < 0.05, "jump of {} at {ms} ms", (p - last).abs());
            last = p;
        }
        let end = after(mid, 6000);
        sim.t.refresh(end);
        assert!(near(sim.t.position(end), 2.0, EPS));
    }

    #[test]
    fn reversing_a_running_spring_is_continuous_and_returns_to_origin() {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        t.track(0.4, 1.0, 3, &band());
        t.release(now, 1, 1.0, params()); // heading to B
        let mid = after(now, 50);
        let p = t.position(mid);
        let v = t.velocity(mid);
        assert!(v > 0.0);
        // reverse: head back to A, carrying the current velocity
        t.release(mid, 0, v, params());
        assert!(near(t.position(mid), p, 1e-9));
        assert!(near(t.velocity(mid), v, 1e-9));
        assert_eq!(t.target(), Some(0));
        let end = after(mid, 6000);
        assert!(t.refresh(end));
        assert!(near(t.position(end), 0.0, EPS));
    }

    #[test]
    fn retarget_timed_is_continuous_in_position_and_velocity() {
        let now = t0();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 3, Duration::from_millis(300));
        let mid = after(now, 110);
        let p = t.position(mid);
        let v = t.velocity(mid);
        t.retarget_timed(mid, 1, Duration::from_millis(200));
        assert!(near(t.position(mid), p, 1e-9), "position continuous");
        assert!(near(t.velocity(mid), v, 1e-9), "velocity continuous");
        let end = after(mid, 400);
        assert!(t.refresh(end));
        assert!(near(t.position(end), 1.0, EPS));
    }

    #[test]
    fn retarget_while_tracking_uses_the_tracked_position() {
        let now = t0();
        let mut t = Transition::new(1);
        t.begin_tracking(now, 1);
        t.track(0.35, 0.8, 3, &band());
        let p = t.position(now);
        t.retarget_timed(now, 3, Duration::from_millis(200));
        assert!(near(t.position(now), p, 1e-9));
        assert!(near(t.velocity(now), 0.8, 1e-9));
    }

    #[test]
    fn rapid_retargeting_never_jumps_and_converges_to_the_last_target() {
        // deterministic pseudo random sequence (no external crates)
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut now = t0();
        let mut t = Transition::new(0);
        let mut last_target = 0usize;
        for step in 0..400 {
            now += Duration::from_millis(next() % 90);
            let was_idle = t.is_idle();
            let before_p = t.position(now);
            let before_v = t.velocity(now);
            let target = (next() % 6) as usize;
            last_target = target;
            let kind = next() % 3;
            match kind {
                0 => t.retarget_timed(now, target, Duration::from_millis(200)),
                1 => t.release(now, target, before_v, params()),
                // a new drag: from an animation it adopts the visual state; from rest it starts
                // on the logical active workspace (here `target`), so only check the former
                _ => t.begin_tracking(now, target),
            }
            if kind != 2 || !was_idle {
                assert!(
                    near(t.position(now), before_p, 1e-6),
                    "step {step} kind {kind}: position jumped from {before_p} to {}",
                    t.position(now)
                );
            }
            if kind == 2 && !was_idle {
                assert!(
                    near(t.velocity(now), before_v, 1e-6),
                    "step {step}: velocity jumped from {before_v} to {}",
                    t.velocity(now)
                );
            }
            t.refresh(now);
        }
        // finish with an explicit retarget and let everything settle
        t.retarget_timed(now, last_target, Duration::from_millis(200));
        let end = now + Duration::from_secs(30);
        assert!(t.refresh(end));
        assert!(t.is_idle());
        assert!(near(t.position(end), last_target as f64, EPS));
    }

    #[test]
    fn snap_cancels_any_animation_immediately() {
        let now = t0();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 3, Duration::from_millis(500));
        assert!(t.is_animating(after(now, 100)));
        t.snap(2);
        assert!(t.is_idle());
        assert!(!t.is_animating(after(now, 100)));
        assert!(near(t.position(after(now, 100)), 2.0, EPS));
        assert_eq!(t.velocity(after(now, 100)), 0.0);
    }

    #[test]
    fn release_onto_the_current_position_with_no_velocity_snaps() {
        let now = t0();
        let mut t = Transition::new(2);
        t.release(now, 2, 0.0, params());
        assert!(t.is_idle());
    }

    // =========================================================================================
    // view generation
    // =========================================================================================

    fn view_at(position: f64, count: usize) -> View {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        // a huge band limit so the rubber band does not alter the test positions in range
        let wide = RubberBand {
            stiffness: 1.0,
            limit: 100.0,
        };
        t.track(position, 0.0, count.max(1) - 1, &wide);
        t.view(now, count)
    }

    #[test]
    fn view_exactly_on_workspaces_has_no_secondary() {
        for (pos, primary) in [(0.0, 0usize), (1.0, 1), (2.0, 2), (3.0, 3)] {
            let v = view_at(pos, 4);
            assert_eq!(v.primary, primary, "pos {pos}");
            assert_eq!(v.secondary, None, "pos {pos}");
            assert_eq!(v.fraction, 0.0);
            assert_eq!(v.edge, 0.0);
        }
    }

    #[test]
    fn view_quarter_half_and_almost_next() {
        let v = view_at(0.25, 4);
        assert_eq!((v.primary, v.secondary), (0, Some(1)));
        assert!((v.fraction - 0.25).abs() < 1e-6);
        assert_eq!(v.edge, 0.0);

        let v = view_at(0.5, 4);
        assert_eq!((v.primary, v.secondary), (0, Some(1)));
        assert!((v.fraction - 0.5).abs() < 1e-6);

        let v = view_at(0.99, 4);
        assert_eq!((v.primary, v.secondary), (0, Some(1)));
        assert!((v.fraction - 0.99).abs() < 1e-6);
    }

    #[test]
    fn view_past_one_uses_the_next_pair() {
        let v = view_at(1.25, 4);
        assert_eq!((v.primary, v.secondary), (1, Some(2)));
        assert!((v.fraction - 0.25).abs() < 1e-6);
    }

    #[test]
    fn view_pair_is_always_adjacent_and_fraction_in_range() {
        let mut p = 0.0;
        while p <= 3.0 {
            let v = view_at(p, 4);
            if let Some(s) = v.secondary {
                assert_eq!(s, v.primary + 1, "pos {p}");
                assert!(v.fraction > 0.0 && v.fraction < 1.0, "pos {p}");
            } else {
                assert_eq!(v.fraction, 0.0, "pos {p}");
            }
            assert!(v.primary <= 3);
            p += 0.0137;
        }
    }

    #[test]
    fn view_below_first_workspace_reports_negative_edge() {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        t.track(-0.4, -1.0, 3, &band());
        let v = t.view(now, 4);
        assert_eq!(v.primary, 0);
        assert_eq!(v.secondary, None);
        assert_eq!(v.fraction, 0.0);
        assert!(v.edge < 0.0, "edge {}", v.edge);
    }

    #[test]
    fn view_above_last_workspace_reports_positive_edge() {
        let now = t0();
        let mut t = Transition::new(3);
        t.begin_tracking(now, 3);
        t.track(0.4, 1.0, 3, &band());
        let v = t.view(now, 4);
        assert_eq!(v.primary, 3);
        assert_eq!(v.secondary, None);
        assert_eq!(v.fraction, 0.0);
        assert!(v.edge > 0.0, "edge {}", v.edge);
    }

    #[test]
    fn view_with_a_single_workspace() {
        let now = t0();
        let mut t = Transition::new(0);
        let v = t.view(now, 1);
        assert_eq!((v.primary, v.secondary, v.fraction, v.edge), (0, None, 0.0, 0.0));
        t.begin_tracking(now, 0);
        t.track(0.3, 1.0, 0, &band());
        let v = t.view(now, 1);
        assert_eq!((v.primary, v.secondary), (0, None));
        assert!(v.edge > 0.0);
        t.track(-0.3, -1.0, 0, &band());
        let v = t.view(now, 1);
        assert_eq!((v.primary, v.secondary), (0, None));
        assert!(v.edge < 0.0);
    }

    #[test]
    fn view_clamps_a_zero_workspace_count_to_one() {
        let now = t0();
        let t = Transition::new(0);
        let v = t.view(now, 0);
        assert_eq!((v.primary, v.secondary), (0, None));
    }

    #[test]
    fn view_during_animation_tracks_the_visual_position() {
        let now = t0();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 1, Duration::from_millis(200));
        let v = t.view(after(now, 100), 3);
        assert_eq!((v.primary, v.secondary), (0, Some(1)));
        assert!((v.fraction - 0.5).abs() < 1e-3);
        let v = t.view(after(now, 250), 3);
        assert_eq!((v.primary, v.secondary), (1, None));
    }

    // =========================================================================================
    // rubber band
    // =========================================================================================

    #[test]
    fn first_workspace_swipe_back_is_resisted_and_bounded() {
        let now = t0();
        let b = band();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        t.track(-0.2, -1.0, 3, &b);
        let p = t.position(now);
        assert!(p < 0.0, "follows the finger a little: {p}");
        assert!(p > -0.2, "but with resistance: {p}");
        assert_eq!(t.view(now, 4).secondary, None);
    }

    #[test]
    fn last_workspace_swipe_forward_is_resisted_and_bounded() {
        let now = t0();
        let b = band();
        let mut t = Transition::new(3);
        t.begin_tracking(now, 3);
        t.track(0.2, 1.0, 3, &b);
        let p = t.position(now);
        assert!(p > 3.0 && p < 3.2, "p {p}");
        assert!(t.view(now, 4).edge > 0.0);
    }

    #[test]
    fn overshoot_is_always_bounded_by_the_band_limit() {
        let now = t0();
        let b = band();
        for far in [0.5, 1.0, 5.0, 100.0, 1.0e6] {
            let mut t = Transition::new(0);
            t.begin_tracking(now, 0);
            t.track(-far, -1.0, 3, &b);
            assert!(t.position(now) > -b.limit, "far {far}: {}", t.position(now));
            let mut t = Transition::new(3);
            t.begin_tracking(now, 3);
            t.track(far, 1.0, 3, &b);
            assert!(t.position(now) < 3.0 + b.limit, "far {far}: {}", t.position(now));
        }
    }

    #[test]
    fn rubber_band_is_monotonic() {
        let now = t0();
        let b = band();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        let mut previous = 0.0;
        let mut delta = 0.0;
        while delta > -3.0 {
            t.track(delta, -1.0, 3, &b);
            let p = t.position(now);
            assert!(p <= previous + 1e-12, "delta {delta}: {p} after {previous}");
            previous = p;
            delta -= 0.01;
        }
        // same at the other end
        let mut t = Transition::new(3);
        t.begin_tracking(now, 3);
        let mut previous = 3.0;
        let mut delta = 0.0;
        while delta < 3.0 {
            t.track(delta, 1.0, 3, &b);
            let p = t.position(now);
            assert!(p >= previous - 1e-12, "delta {delta}: {p} after {previous}");
            previous = p;
            delta += 0.01;
        }
    }

    #[test]
    fn inside_the_workspaces_the_band_does_not_change_the_position() {
        let now = t0();
        let mut t = Transition::new(1);
        t.begin_tracking(now, 1);
        t.track(0.7, 0.5, 3, &band());
        assert!(near(t.position(now), 1.7, EPS));
        assert!(near(t.velocity(now), 0.5, EPS));
    }

    #[test]
    fn velocity_is_attenuated_in_the_overshoot() {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        t.track(-0.3, -2.0, 3, &band());
        let v = t.velocity(now);
        assert!(v < 0.0 && v > -2.0, "v {v}");
    }

    #[test]
    fn release_from_overshoot_returns_to_the_valid_edge_workspace_without_activating() {
        let now = t0();
        let mut sim = Sim::new(0, 4);
        sim.begin(now);
        sim.drag(-0.6, -1.0);
        assert!(sim.t.position(now) < 0.0);
        sim.finish(now, -1.0);
        assert_eq!(sim.active, 0, "there is no workspace before the first one");
        let later = after(now, 5000);
        assert!(sim.t.refresh(later));
        assert!(near(sim.t.position(later), 0.0, EPS));
        assert_eq!(sim.t.view(later, 4).primary, 0);
        assert_eq!(sim.t.view(later, 4).edge, 0.0);
    }

    #[test]
    fn release_from_overshoot_at_the_last_workspace() {
        let now = t0();
        let mut sim = Sim::new(3, 4);
        sim.begin(now);
        sim.drag(0.6, 1.0);
        assert!(sim.t.position(now) > 3.0);
        sim.finish(now, 1.0);
        assert_eq!(sim.active, 3);
        let later = after(now, 5000);
        assert!(sim.t.refresh(later));
        assert!(near(sim.t.position(later), 3.0, EPS));
    }

    #[test]
    fn overshoot_never_makes_active_invalid() {
        let now = t0();
        let mut sim = Sim::new(0, 3);
        sim.begin(now);
        for step in 1..50 {
            sim.drag(-(step as f64) * 0.1, -2.0);
            assert_eq!(sim.active, 0);
            assert!(sim.t.position(now) > -band().limit);
        }
        sim.finish(now, -3.0);
        assert_eq!(sim.active, 0);
    }

    #[test]
    fn single_workspace_set_resists_both_directions_and_never_activates_another() {
        let now = t0();
        let mut sim = Sim::new(0, 1);
        for (delta, vel) in [(0.5, 2.0), (-0.5, -2.0)] {
            sim.begin(now);
            sim.drag(delta, vel);
            let p = sim.t.position(now);
            assert!(p.abs() < band().limit, "p {p}");
            sim.finish(now, vel);
            assert_eq!(sim.active, 0);
            let later = after(now, 5000);
            assert!(sim.t.refresh(later));
            assert!(near(sim.t.position(later), 0.0, EPS));
        }
    }

    #[test]
    fn two_workspace_set_boundaries() {
        let now = t0();
        let mut sim = Sim::new(0, 2);
        sim.begin(now);
        sim.drag(1.4, 1.0); // 0.4 past the last workspace, banded
        let p = sim.t.position(now);
        assert!(p > 1.0 && p < 1.0 + band().limit, "p {p}");
        sim.finish(now, 1.0);
        assert_eq!(sim.active, 1);
    }

    #[test]
    fn dragging_then_reversing_past_the_origin_works_in_both_directions() {
        let now = t0();
        let mut sim = Sim::new(1, 3);
        sim.begin(now);
        sim.drag(0.6, 1.0);
        sim.drag(-0.5, -1.0); // went back through the origin to the other side
        assert!(near(sim.t.position(now), 0.5, EPS));
        sim.finish(now, -1.0);
        assert_eq!(sim.active, 0);
    }

    // =========================================================================================
    // mapping a view onto the renderer's previous + current model
    // =========================================================================================

    fn pair_at(position: f64, active: usize, count: usize) -> PairView {
        pair_moving(position, active, count, 0)
    }

    fn pair_moving(position: f64, active: usize, count: usize, direction: i32) -> PairView {
        let now = t0();
        let mut t = Transition::new(0);
        t.begin_tracking(now, 0);
        let wide = RubberBand {
            stiffness: 1.0,
            limit: 100.0,
        };
        t.track(position, 0.0, count.max(1) - 1, &wide);
        pair_view(&t.view(now, count), active, count.max(1) - 1, direction)
    }

    #[test]
    fn pair_forward_swipe_matches_the_old_pair_model() {
        // 1 -> 2 at 30%: the old model had previous 1, current 2 (already active), forward
        let p = pair_at(1.3, 2, 4);
        assert_eq!(p.current, 2);
        let prev = p.previous.unwrap();
        assert_eq!((prev.idx, prev.forward), (1, true));
        assert!((prev.percentage - 0.3).abs() < 1e-6);
    }

    #[test]
    fn pair_backward_swipe_matches_the_old_pair_model() {
        // 3 -> 2 at 30%: previous 3 (leaving to the right), current 2, not forward
        let p = pair_at(2.7, 2, 4);
        assert_eq!(p.current, 2);
        let prev = p.previous.unwrap();
        assert_eq!((prev.idx, prev.forward), (3, false));
        assert!((prev.percentage - 0.3).abs() < 1e-6);
    }

    #[test]
    fn pair_cancelled_forward_swipe_matches_the_old_pair_model() {
        // swiped 1 -> 2, cancelled: active is 1 again and the visual position is 1.3 going back
        let p = pair_at(1.3, 1, 4);
        assert_eq!(p.current, 1);
        let prev = p.previous.unwrap();
        assert_eq!((prev.idx, prev.forward), (2, false));
        assert!((prev.percentage - 0.7).abs() < 1e-6);
    }

    #[test]
    fn pair_percentage_runs_from_zero_to_one_as_previous_leaves() {
        // timed switch 1 -> 2: previous is 1, percentage grows with the position
        let mut last = -1.0f32;
        for step in 1..100 {
            let pos = 1.0 + step as f64 / 100.0;
            let p = pair_at(pos, 2, 4);
            let prev = p.previous.unwrap();
            assert!(prev.percentage > last);
            last = prev.percentage;
        }
        assert!(last > 0.98);
    }

    #[test]
    fn pair_without_secondary_has_no_previous() {
        for (pos, active) in [(0.0, 0usize), (2.0, 2), (3.0, 3)] {
            let p = pair_at(pos, active, 4);
            assert_eq!(p, PairView { current: active, previous: None });
        }
    }

    #[test]
    fn pair_when_active_is_not_in_the_drawn_pair_treats_the_higher_as_current() {
        // retargeted mid-flight: drawing 1 and 2 while the logical active workspace is 3
        let p = pair_at(1.4, 3, 4);
        assert_eq!(p.current, 2);
        let prev = p.previous.unwrap();
        assert_eq!((prev.idx, prev.forward), (1, true));
        assert!((prev.percentage - 0.4).abs() < 1e-6);
        // and when it is below the pair
        let p = pair_at(2.6, 0, 4);
        assert_eq!(p.current, 3);
        assert_eq!(p.previous.unwrap().idx, 2);
    }

    #[test]
    fn pair_is_always_two_adjacent_workspaces() {
        let mut pos = 0.0;
        while pos <= 3.0 {
            for active in 0..4 {
                let p = pair_at(pos, active, 4);
                if let Some(prev) = p.previous {
                    assert_eq!((prev.idx as i64 - p.current as i64).abs(), 1, "pos {pos} active {active}");
                    assert!(prev.percentage > 0.0 && prev.percentage < 1.0);
                    assert_eq!(prev.forward, prev.idx < p.current);
                }
            }
            pos += 0.0173;
        }
    }

    #[test]
    fn a_timed_shortcut_switch_drawn_through_the_pair_model_starts_and_ends_like_the_old_one() {
        // old model: previous = old workspace, current = new one, percentage 0 -> 1
        let now = t0();
        let mut t = Transition::new(1);
        t.retarget_timed(now, 2, Duration::from_millis(200));
        let draw = |ms: u64| {
            let at = after(now, ms);
            pair_view(&t.view(at, 4), 2, 3, t.direction(at))
        };
        // FIRST FRAME (position exactly on workspace 1): already previous 1 / current 2
        let start = draw(0);
        assert_eq!(start.current, 2, "current must be the workspace being entered from frame 0");
        let prev = start.previous.expect("previous must be drawn from frame 0");
        assert_eq!((prev.idx, prev.forward), (1, true));
        assert_eq!(prev.percentage, 0.0);
        // a moment later: same pair, percentage small
        let early = draw(10);
        assert_eq!(early.current, 2);
        let prev = early.previous.unwrap();
        assert_eq!((prev.idx, prev.forward), (1, true));
        assert!(prev.percentage > 0.0 && prev.percentage < 0.05);
        // at the end only the new workspace remains
        assert_eq!(draw(250), PairView { current: 2, previous: None });
    }

    #[test]
    fn first_frame_pair_is_the_same_whichever_way_the_switch_goes() {
        let now = t0();
        // downward: 2 -> 1, the old model had previous 2 (leaving to the right), current 1
        let mut t = Transition::new(2);
        t.retarget_timed(now, 1, Duration::from_millis(200));
        let first = pair_view(&t.view(now, 4), 1, 3, t.direction(now));
        assert_eq!(first.current, 1);
        let prev = first.previous.unwrap();
        assert_eq!((prev.idx, prev.forward), (2, false));
        assert_eq!(prev.percentage, 0.0);
        // and it continues with the same pair
        let later = after(now, 40);
        let next = pair_view(&t.view(later, 4), 1, 3, t.direction(later));
        assert_eq!(next.current, 1);
        assert_eq!(next.previous.unwrap().idx, 2);
    }

    #[test]
    fn first_frame_pair_for_a_non_adjacent_switch_uses_the_pair_in_the_direction_of_motion() {
        let now = t0();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 3, Duration::from_millis(200));
        // first frame: on workspace 0, moving up: pair (0, 1); active (3) is in neither so the
        // higher one of the pair is current, exactly like every later frame of this segment
        let first = pair_view(&t.view(now, 4), 3, 3, t.direction(now));
        assert_eq!(first.current, 1);
        assert_eq!(first.previous.unwrap().idx, 0);
        let second = pair_view(&t.view(after(now, 5), 4), 3, 3, t.direction(after(now, 5)));
        assert_eq!(second.current, first.current, "no change of primary between frame 0 and frame 1");
        assert_eq!(second.previous.unwrap().idx, 0);
    }

    #[test]
    fn rendering_and_hit_testing_use_the_same_pair_on_every_frame_of_a_switch() {
        // both go through pair_view with the same inputs; check no frame has a different primary
        // than the frames around it, for all directions and distances
        for (from, to) in [(0usize, 1usize), (1, 0), (1, 2), (3, 2), (0, 3), (3, 0), (1, 3), (2, 0)] {
            let now = t0();
            let mut t = Transition::new(from);
            t.retarget_timed(now, to, Duration::from_millis(200));
            for ms in 0..=200u64 {
                let at = after(now, ms);
                let pair = pair_view(&t.view(at, 4), to, 3, t.direction(at));
                if let Some(prev) = pair.previous {
                    assert_eq!((prev.idx as i64 - pair.current as i64).abs(), 1, "{from}->{to} at {ms} ms");
                }
            }
            // the very first frame must already be a pair (the old workspace is leaving)
            let first = pair_view(&t.view(now, 4), to, 3, t.direction(now));
            assert!(first.previous.is_some(), "{from}->{to}: first frame has no previous");
        }
    }

    #[test]
    fn direction_is_zero_when_idle_tracking_or_arrived() {
        let now = t0();
        let mut t = Transition::new(1);
        assert_eq!(t.direction(now), 0);
        t.begin_tracking(now, 1);
        assert_eq!(t.direction(now), 0);
        t.retarget_timed(now, 3, Duration::from_millis(200));
        assert_eq!(t.direction(now), 1);
        assert_eq!(t.direction(after(now, 100)), 1);
        assert_eq!(t.direction(after(now, 300)), 0);
        t.snap(2);
        t.retarget_timed(after(now, 400), 0, Duration::from_millis(200));
        assert_eq!(t.direction(after(now, 400)), -1);
    }

    #[test]
    fn pair_view_without_motion_on_a_workspace_is_unchanged() {
        // not switching: only the one workspace, even at the first or last index
        for (pos, active) in [(0.0, 0usize), (2.0, 2), (3.0, 3)] {
            assert_eq!(pair_at(pos, active, 4), PairView { current: active, previous: None });
        }
    }

    #[test]
    fn first_frame_pair_is_clamped_at_the_ends() {
        // moving up from the last workspace or down from the first cannot happen, but must not panic
        let now = t0();
        let t = Transition::new(3);
        let v = t.view(now, 4);
        assert_eq!(pair_view(&v, 3, 3, 1), PairView { current: 3, previous: None });
        let t = Transition::new(0);
        let v = t.view(now, 4);
        assert_eq!(pair_view(&v, 0, 3, -1), PairView { current: 0, previous: None });
    }
}

/// Stage B2: the transition keeps referring to the same workspaces (by identity) when the
/// workspace list changes. Workspaces are named by letters standing in for handles; every test
/// checks *which workspaces are drawn*, not just the numbers.
#[cfg(test)]
mod remap_tests {
    use super::*;

    const TOL: f32 = 1e-5;

    fn spring() -> SpringParams {
        SpringParams::new(1.0, 600.0, 0.0001)
    }

    fn band() -> RubberBand {
        RubberBand {
            stiffness: 0.5,
            limit: 0.25,
        }
    }

    fn decide_params() -> DecideParams {
        DecideParams {
            velocity_threshold: 0.3,
            position_threshold: 0.5,
        }
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    /// The workspaces drawn at `now`: (primary, secondary, fraction, edge overshoot).
    fn drawn(t: &Transition, now: Instant, list: &[char]) -> (char, Option<char>, f32, f32) {
        let v = t.view(now, list.len());
        (list[v.primary], v.secondary.map(|i| list[i]), v.fraction, v.edge)
    }

    fn same_drawing(a: (char, Option<char>, f32, f32), b: (char, Option<char>, f32, f32)) {
        assert_eq!((a.0, a.1), (b.0, b.1), "different workspaces drawn: {a:?} vs {b:?}");
        assert!((a.2 - b.2).abs() < TOL, "fraction moved: {a:?} vs {b:?}");
        assert!((a.3 - b.3).abs() < TOL, "edge moved: {a:?} vs {b:?}");
    }

    /// Apply a list change. `active` is the logical active workspace after the change (what the
    /// shell passes).
    fn change(t: &mut Transition, now: Instant, before: &[char], after: &[char], active: char) -> RemapOutcome {
        let map = index_map(before, after);
        let new_active = after.iter().position(|h| *h == active).expect("active is in the new list");
        t.remap(now, &map, after.len(), new_active)
    }

    /// Tracking from `list[active]` with a total finger delta of `delta` workspaces.
    fn tracking(now: Instant, active: usize, delta: f64, count: usize) -> Transition {
        let mut t = Transition::new(active);
        t.begin_tracking(now, active);
        t.track(delta, 0.0, count - 1, &band());
        t
    }

    const ABCD: [char; 4] = ['A', 'B', 'C', 'D'];

    #[test]
    fn index_map_follows_identity() {
        assert_eq!(index_map(&['A', 'B', 'C'], &['B', 'C']), vec![None, Some(0), Some(1)]);
        assert_eq!(index_map(&['A', 'B'], &['B', 'X', 'A']), vec![Some(2), Some(0)]);
        assert_eq!(index_map::<char>(&[], &['A']), Vec::<Option<usize>>::new());
    }

    // 1
    #[test]
    fn remove_before_the_pair_keeps_drawing_the_same_pair() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4); // B 0.4 of the way to C
        let before = drawn(&t, now, &ABCD);
        assert_eq!((before.0, before.1), ('B', Some('C')));
        let after = ['B', 'C', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'B'), RemapOutcome::Translated(-1));
        same_drawing(drawn(&t, now, &after), before);
        assert!(t.is_tracking());
    }

    // 2 (a referenced workspace is removed)
    /// Documented behavior: the primary (here also the active workspace) is gone, so the drag is
    /// re-based on whatever the shell made active. It must not silently show the old numbers
    /// (which would now be C <-> D).
    #[test]
    fn removing_the_primary_falls_back_to_the_new_active_workspace() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4);
        let after = ['A', 'C', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'C'), RemapOutcome::FellBackToActive);
        assert_eq!(drawn(&t, now, &after), ('C', None, 0.0, 0.0));
        assert!(t.is_tracking(), "the gesture still belongs to the user");
    }

    // 3 (a referenced workspace is removed)
    /// Documented behavior: dragging B toward C and C disappears. The drag jumps back onto B
    /// (the active workspace) and keeps following the fingers from there: it never starts
    /// showing D in C's place at the old fraction.
    #[test]
    fn removing_the_secondary_falls_back_and_tracking_continues_relative() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4);
        let after = ['A', 'B', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'B'), RemapOutcome::FellBackToActive);
        assert_eq!(drawn(&t, now, &after), ('B', None, 0.0, 0.0));
        // the fingers move another 0.2 (total 0.6): 0.2 past B, toward D
        t.track(0.6, 0.0, after.len() - 1, &band());
        let d = drawn(&t, now, &after);
        assert_eq!((d.0, d.1), ('B', Some('D')));
        assert!((d.2 - 0.2).abs() < TOL);
    }

    // 4
    #[test]
    fn remove_after_the_pair_changes_nothing() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4);
        let before = drawn(&t, now, &ABCD);
        let after = ['A', 'B', 'C'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'B'), RemapOutcome::Unchanged);
        same_drawing(drawn(&t, now, &after), before);
    }

    // 5
    #[test]
    fn remove_while_tracking_keeps_following_the_same_workspaces() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.2, 4);
        let mut reference = t;
        let after = ['B', 'C', 'D'];
        change(&mut t, now, &ABCD, &after, 'B');
        for delta in [0.3, 0.5, 0.9, 1.2] {
            t.track(delta, 1.0, after.len() - 1, &band());
            reference.track(delta, 1.0, ABCD.len() - 1, &band());
            same_drawing(drawn(&t, now, &after), drawn(&reference, now, &ABCD));
        }
    }

    // 6
    #[test]
    fn remove_while_the_release_spring_runs_keeps_the_motion_exact() {
        let now = Instant::now();
        let list = ['A', 'B', 'C', 'D', 'E'];
        let mut t = tracking(now, 1, 0.6, list.len());
        t.release(now, 2, 1.5, spring()); // settling on C
        let reference = t;
        let later = at(now, 40);
        let after = ['B', 'C', 'D', 'E'];
        assert_eq!(change(&mut t, later, &list, &after, 'C'), RemapOutcome::Translated(-1));
        assert_eq!(t.target(), Some(1)); // still C
        for ms in [40, 60, 100, 200, 400] {
            same_drawing(drawn(&t, at(now, ms), &after), drawn(&reference, at(now, ms), &list));
            assert!((t.velocity(at(now, ms)) - reference.velocity(at(now, ms))).abs() < 1e-9);
        }
        t.refresh(at(now, 5000));
        assert!(t.is_idle());
        assert_eq!(drawn(&t, at(now, 5000), &after), ('C', None, 0.0, 0.0));
    }

    // 7
    #[test]
    fn insert_before_the_pair_keeps_drawing_the_same_pair() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4);
        let before = drawn(&t, now, &ABCD);
        let after = ['X', 'A', 'B', 'C', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'B'), RemapOutcome::Translated(1));
        same_drawing(drawn(&t, now, &after), before);
    }

    // 8
    #[test]
    fn insert_after_the_pair_changes_nothing() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4);
        let before = drawn(&t, now, &ABCD);
        let after = ['A', 'B', 'C', 'X', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'B'), RemapOutcome::Unchanged);
        same_drawing(drawn(&t, now, &after), before);
    }

    /// Documented behavior: a workspace inserted *between* the two drawn workspaces (the shell
    /// does this when a workspace is migrated to `active + 1`) breaks the pair, so the drag is
    /// re-based on the active workspace instead of drawing B <-> X.
    #[test]
    fn insert_inside_the_pair_falls_back_to_the_active_workspace() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4);
        let after = ['A', 'B', 'X', 'C', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'B'), RemapOutcome::FellBackToActive);
        assert_eq!(drawn(&t, now, &after), ('B', None, 0.0, 0.0));
    }

    // 9
    #[test]
    fn reordering_a_workspace_outside_the_pair_keeps_the_pair() {
        let now = Instant::now();
        let list = ['A', 'B', 'C', 'D', 'E'];
        let mut t = tracking(now, 2, 0.4, list.len()); // C 0.4 of the way to D
        let before = drawn(&t, now, &list);
        // A moved to the end: everything before the pair shifts left
        let moved = ['B', 'C', 'D', 'E', 'A'];
        assert_eq!(change(&mut t, now, &list, &moved, 'C'), RemapOutcome::Translated(-1));
        same_drawing(drawn(&t, now, &moved), before);
        // E moved to the front: everything shifts right
        let moved2 = ['A', 'B', 'C', 'D', 'E'];
        let mut t2 = tracking(now, 2, 0.4, list.len());
        let front = ['E', 'A', 'B', 'C', 'D'];
        assert_eq!(change(&mut t2, now, &moved2, &front, 'C'), RemapOutcome::Translated(1));
        same_drawing(drawn(&t2, now, &front), before);
    }

    // 10
    #[test]
    fn transition_at_the_first_workspace() {
        let now = Instant::now();
        let mut t = tracking(now, 0, 0.3, 4); // A toward B
        let before = drawn(&t, now, &ABCD);
        let after = ['A', 'B', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'A'), RemapOutcome::Unchanged);
        same_drawing(drawn(&t, now, &after), before);
        let after2 = ['X', 'A', 'B', 'D'];
        assert_eq!(change(&mut t, now, &after, &after2, 'A'), RemapOutcome::Translated(1));
        same_drawing(drawn(&t, now, &after2), before);
    }

    // 11
    #[test]
    fn transition_at_the_last_workspace() {
        let now = Instant::now();
        let mut t = tracking(now, 3, 0.0, 4); // resting on D under the fingers
        let after = ['A', 'C', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'D'), RemapOutcome::Translated(-1));
        assert_eq!(drawn(&t, now, &after), ('D', None, 0.0, 0.0));
        t.track(-0.5, 0.0, after.len() - 1, &band());
        let d = drawn(&t, now, &after);
        assert_eq!((d.0, d.1), ('C', Some('D')));
    }

    // 12
    #[test]
    fn rubber_band_below_the_first_workspace_keeps_its_overshoot() {
        let now = Instant::now();
        let mut t = tracking(now, 0, -0.5, 4);
        let before = drawn(&t, now, &ABCD);
        assert_eq!((before.0, before.1), ('A', None));
        assert!(before.3 < 0.0);
        let after = ['A', 'B', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'A'), RemapOutcome::Unchanged);
        same_drawing(drawn(&t, now, &after), before);
    }

    // 13
    #[test]
    fn rubber_band_above_the_last_workspace_keeps_its_overshoot() {
        let now = Instant::now();
        let mut t = tracking(now, 3, 0.5, 4);
        let before = drawn(&t, now, &ABCD);
        assert_eq!((before.0, before.1), ('D', None));
        assert!(before.3 > 0.0);
        let after = ['B', 'C', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'D'), RemapOutcome::Translated(-1));
        same_drawing(drawn(&t, now, &after), before);
    }

    // 14
    #[test]
    fn idle_transition_after_a_change_stays_on_its_workspace() {
        let now = Instant::now();
        let mut t = Transition::new(2); // idle on C
        let after = ['B', 'C', 'D'];
        assert_eq!(change(&mut t, now, &ABCD, &after, 'C'), RemapOutcome::Translated(-1));
        assert!(t.is_idle());
        assert_eq!(drawn(&t, now, &after), ('C', None, 0.0, 0.0));
        // its own workspace removed: idle on whatever the shell made active
        let after2 = ['B', 'D'];
        assert_eq!(change(&mut t, now, &after, &after2, 'D'), RemapOutcome::FellBackToActive);
        assert!(t.is_idle());
        assert_eq!(drawn(&t, now, &after2), ('D', None, 0.0, 0.0));
    }

    // 15
    #[test]
    fn cancel_after_a_change_returns_to_the_origin_workspace() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.4, 4);
        let after = ['B', 'C', 'D'];
        change(&mut t, now, &ABCD, &after, 'B');
        t.cancel(now, 0, spring()); // the shell's origin is B, now at index 0
        let mid = drawn(&t, at(now, 30), &after);
        assert_eq!((mid.0, mid.1), ('B', Some('C')));
        t.refresh(at(now, 5000));
        assert_eq!(drawn(&t, at(now, 5000), &after), ('B', None, 0.0, 0.0));
    }

    // 16
    #[test]
    fn finish_after_a_change_settles_on_the_intended_workspace() {
        let now = Instant::now();
        let mut t = tracking(now, 1, 0.7, 4); // B 0.7 of the way to C
        let after = ['B', 'C', 'D'];
        change(&mut t, now, &ABCD, &after, 'B');
        let target = t.finish(now, 0, 0.0, after.len() - 1, decide_params(), spring());
        assert_eq!(after[target], 'C');
        t.refresh(at(now, 5000));
        assert_eq!(drawn(&t, at(now, 5000), &after), ('C', None, 0.0, 0.0));
    }

    // 17
    #[test]
    fn retarget_after_a_change_starts_from_the_same_visual_state() {
        let now = Instant::now();
        let list = ['A', 'B', 'C', 'D', 'E'];
        let mut t = Transition::new(1);
        t.retarget_timed(now, 3, Duration::from_millis(200)); // B -> D
        let mid = at(now, 60);
        let seen = drawn(&t, mid, &list);
        let after = ['B', 'C', 'D', 'E'];
        assert_eq!(change(&mut t, mid, &list, &after, 'D'), RemapOutcome::Translated(-1));
        same_drawing(drawn(&t, mid, &after), seen);
        let v = t.velocity(mid);
        // a shortcut back to B, now index 0
        t.retarget_timed(mid, 0, Duration::from_millis(200));
        same_drawing(drawn(&t, mid, &after), seen);
        assert!((t.velocity(mid) - v).abs() < 1e-9, "velocity is carried over");
        t.refresh(at(mid, 1000));
        assert_eq!(drawn(&t, at(mid, 1000), &after), ('B', None, 0.0, 0.0));
    }

    // 18
    #[test]
    fn non_adjacent_timed_switch_with_changes_outside_and_inside_its_path() {
        let now = Instant::now();
        let list = ['A', 'B', 'C', 'D', 'E'];
        let start = || {
            let mut t = Transition::new(0);
            t.retarget_timed(now, 3, Duration::from_millis(200)); // A -> D, passing B and C
            t
        };
        let mid = at(now, 100);

        // a workspace after the target: nothing changes
        let mut t = start();
        let seen = drawn(&t, mid, &list);
        let after = ['A', 'B', 'C', 'D'];
        assert_eq!(change(&mut t, mid, &list, &after, 'D'), RemapOutcome::Unchanged);
        same_drawing(drawn(&t, mid, &after), seen);

        // a workspace already passed (behind the current position): exact translation
        let mut t = start();
        assert!(t.position(mid) > 1.0);
        let seen = drawn(&t, mid, &list);
        let after = ['B', 'C', 'D', 'E'];
        assert_eq!(change(&mut t, mid, &list, &after, 'D'), RemapOutcome::Translated(-1));
        same_drawing(drawn(&t, mid, &after), seen);
        assert_eq!(t.target(), Some(2)); // D

        // a workspace still ahead on the path (C) is removed. Documented behavior: the
        // animation cannot keep its path, so it lands on its destination (the active workspace)
        // at once instead of sliding through a different workspace in C's place.
        let mut t = start();
        let after = ['A', 'B', 'D', 'E'];
        assert_eq!(change(&mut t, mid, &list, &after, 'D'), RemapOutcome::FellBackToActive);
        assert!(t.is_idle());
        assert_eq!(drawn(&t, mid, &after), ('D', None, 0.0, 0.0));
    }

    /// Documented behavior: the target of a running animation is removed. The shell has already
    /// picked a new active workspace; the transition snaps there.
    #[test]
    fn removing_the_animation_target_snaps_to_the_new_active_workspace() {
        let now = Instant::now();
        let mut t = Transition::new(1);
        t.retarget_timed(now, 2, Duration::from_millis(200)); // B -> C
        let after = ['A', 'B', 'D'];
        assert_eq!(change(&mut t, at(now, 50), &ABCD, &after, 'B'), RemapOutcome::FellBackToActive);
        assert!(t.is_idle());
        assert_eq!(drawn(&t, at(now, 50), &after), ('B', None, 0.0, 0.0));
    }

    #[test]
    fn degenerate_inputs_never_index_out_of_bounds() {
        let now = Instant::now();
        // empty old list
        let mut t = Transition::new(0);
        assert_eq!(t.remap(now, &[], 2, 1), RemapOutcome::FellBackToActive);
        assert_eq!(t.view(now, 2).primary, 1);
        // empty new list
        let mut t = Transition::new(0);
        assert_eq!(t.remap(now, &[None], 0, 0), RemapOutcome::FellBackToActive);
        // new_active out of range is clamped
        let mut t = Transition::new(3);
        assert_eq!(t.remap(now, &[Some(0), None, None, None], 1, 9), RemapOutcome::FellBackToActive);
        assert_eq!(t.view(now, 1).primary, 0);
        // a map pointing past the new list is treated as removed
        let mut t = Transition::new(0);
        assert_eq!(t.remap(now, &[Some(5)], 2, 0), RemapOutcome::FellBackToActive);
    }
}
