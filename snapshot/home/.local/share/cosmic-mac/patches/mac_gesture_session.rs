// SPDX-License-Identifier: GPL-3.0-only
//
// cosmic-mac Stage A3: gesture ownership and the compositor-owned swipe session.
//
// PURE: no smithay, Wayland, Shell, D-Bus or rendering. It decides who owns a swipe, turns the
// recognizer's output into workspace / overview actions and converts pixels to workspace
// units. The shell and input glue only apply the `Action`s it returns.
#![allow(dead_code)]

//! # Ownership
//! Decided **once, at the start of a swipe** ([`decide_owner`] via [`Owner::begin`]) and never
//! transferred:
//!
//! ```text
//! 3 or 4 fingers AND no pointer grab        -> Compositor owns the gesture
//! anything else                             -> Client (forwarded to the focused client)
//! ```
//! A compositor session that is still alive when a new swipe begins (it should not be: libinput
//! always ends a gesture) is aborted safely first; the caller applies the returned abort action.
//!
//! # What a compositor-owned swipe does
//! With the usual horizontal workspace layout:
//! - horizontal: follows the fingers across workspaces (see `workspace_transition`);
//! - vertical, fingers up: shows the overview, once, when the distance threshold is crossed;
//! - vertical, fingers down: hides the overview, once, when it is open;
//! - every other combination (up while it is open, down while it is closed, horizontal while it
//!   is open) is a deliberate no-op that is **not** forwarded to any client.
//! With a vertical workspace layout the vertical axis switches workspaces and horizontal
//! movement is ignored.
//!
//! # Units and signs
//! The recognizer works in pixels (right/down positive). Workspace deltas and velocities are
//! `pixels / swipe_length` (never the output width) and positive means "toward a higher
//! workspace index". With natural scrolling the content follows the fingers, so fingers moving
//! left (or up in a vertical layout) mean a higher index; without natural scrolling the sign is
//! the opposite. This is exactly the mapping the stock code used.
//!
//! All numbers in [`Tuning`]'s `Default` are PROVISIONAL placeholders, to be set from real
//! touchpad recordings.

use std::time::Duration;

use super::recognizer::{Axis, Input, Output as Recognized, Recognizer, RecognizerConfig};
use crate::shell::workspace_transition::{pixel_velocity_to_workspaces, pixels_to_workspaces};

// ---------------------------------------------------------------------------------------------
// tuning
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    pub recognizer: RecognizerConfig,
    /// Finger travel (px) for one whole workspace. Also the unit for velocity.
    pub swipe_length: f64,
    /// Vertical distance (px) that triggers the overview once.
    pub vertical_trigger: f64,
    /// Release speed (workspaces per second) at which the direction of motion decides.
    pub velocity_threshold: f64,
    /// Distance (workspaces) from the origin that must be passed to move when slow.
    pub position_threshold: f64,
    /// Rubber band (workspace units) past the first and last workspace.
    pub band_stiffness: f64,
    pub band_limit: f64,
}

impl Default for Tuning {
    /// PROVISIONAL placeholder values, to be replaced from real touchpad recordings.
    fn default() -> Self {
        Tuning {
            recognizer: RecognizerConfig::default(),
            swipe_length: 150.0,
            vertical_trigger: 60.0,
            velocity_threshold: 0.3,
            position_threshold: 0.5,
            band_stiffness: 0.5,
            band_limit: 0.25,
        }
    }
}

impl Tuning {
    /// Defaults with development overrides read through `get` (environment variables in the
    /// compositor). Out-of-range values are clamped to something safe.
    pub fn from_lookup(get: impl Fn(&str) -> Option<f64>) -> Tuning {
        let d = Tuning::default();
        let value = |name: &str, default: f64, min: f64| {
            get(name).filter(|v| v.is_finite()).unwrap_or(default).max(min)
        };
        Tuning {
            recognizer: RecognizerConfig {
                dead_zone: value("COSMIC_MAC_GESTURE_DEAD_ZONE", d.recognizer.dead_zone, 0.0),
                axis_ratio: value("COSMIC_MAC_GESTURE_AXIS_RATIO", d.recognizer.axis_ratio, 1.0),
                ..d.recognizer
            },
            swipe_length: value("COSMIC_MAC_SWIPE_LENGTH", d.swipe_length, 10.0),
            vertical_trigger: value("COSMIC_MAC_GESTURE_VERTICAL_TRIGGER", d.vertical_trigger, 1.0),
            velocity_threshold: value("COSMIC_MAC_GESTURE_VELOCITY", d.velocity_threshold, 0.0),
            position_threshold: value("COSMIC_MAC_GESTURE_POSITION", d.position_threshold, 0.0),
            band_stiffness: value("COSMIC_MAC_GESTURE_BAND_STIFFNESS", d.band_stiffness, 0.01),
            band_limit: value("COSMIC_MAC_GESTURE_BAND_LIMIT", d.band_limit, 0.01),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ownership
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The compositor owns the gesture. `cancel_existing` is set when a compositor session was
    /// still alive and must be aborted first.
    Compositor { cancel_existing: bool },
    /// The gesture is forwarded to the focused client.
    Client { cancel_existing: bool },
}

/// Who owns a swipe that is beginning. `pointer_grabbed` is `PointerHandle::is_grabbed()`.
pub fn decide_owner(
    fingers: u32,
    pointer_grabbed: bool,
    has_compositor_session: bool,
    config: &RecognizerConfig,
) -> Decision {
    let eligible = fingers >= config.min_fingers && fingers <= config.max_fingers;
    if eligible && !pointer_grabbed {
        Decision::Compositor {
            cancel_existing: has_compositor_session,
        }
    } else {
        Decision::Client {
            cancel_existing: has_compositor_session,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKind {
    None,
    Client,
    Compositor,
}

/// The owner of the swipe in progress. `T` is whatever the caller needs to remember about a
/// compositor-owned swipe (the output it began on).
#[derive(Debug, Default)]
pub enum Owner<T> {
    #[default]
    None,
    Client,
    Compositor {
        session: Session,
        extra: T,
    },
}

impl<T> Owner<T> {
    pub fn kind(&self) -> OwnerKind {
        match self {
            Owner::None => OwnerKind::None,
            Owner::Client => OwnerKind::Client,
            Owner::Compositor { .. } => OwnerKind::Compositor,
        }
    }

    /// A swipe begins: decide the owner (once) and install it. Returns the previous
    /// compositor session's remains, if one was still alive: its `extra` and the action that
    /// aborts it, which the caller must apply.
    pub fn begin(
        &mut self,
        fingers: u32,
        pointer_grabbed: bool,
        tuning: &Tuning,
        extra: T,
    ) -> Option<(T, Action)> {
        let decision = decide_owner(
            fingers,
            pointer_grabbed,
            matches!(self, Owner::Compositor { .. }),
            &tuning.recognizer,
        );
        let previous = match std::mem::take(self) {
            Owner::Compositor { mut session, extra } => Some((extra, session.abort())),
            _ => None,
        };
        match decision {
            Decision::Compositor { .. } => {
                let mut session = Session::new(*tuning);
                session.begin(fingers, Duration::ZERO);
                *self = Owner::Compositor { session, extra };
            }
            Decision::Client { .. } => *self = Owner::Client,
        }
        previous
    }

    /// The swipe ended (or was invalidated): back to no owner.
    pub fn end(&mut self) {
        *self = Owner::None;
    }
}

// ---------------------------------------------------------------------------------------------
// the compositor-owned swipe
// ---------------------------------------------------------------------------------------------

/// What the compositor knows when an event arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// The touchpad uses natural scrolling.
    pub natural_scroll: bool,
    /// Workspaces are laid out vertically (stock COSMIC option); the default is horizontal.
    pub vertical_layout: bool,
    /// The workspace overview is open right now.
    pub overview_open: bool,
}

/// What the shell/input glue must do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    None,
    /// Follow the fingers. `first` is set on the first tracking update (begin tracking from the
    /// current visual state). `total` is the total movement since the gesture began and
    /// `velocity` the current velocity, both in workspace units (positive = higher index).
    WorkspaceTrack {
        first: bool,
        total: f64,
        velocity: f64,
    },
    /// Fingers lifted normally: pick the target workspace. `velocity` in workspace units/s.
    WorkspaceFinish { velocity: f64 },
    /// Cancelled: animate back to the logical workspace and never activate another one.
    WorkspaceCancel,
    OverviewShow,
    OverviewHide,
}

/// A method call without arguments on the session bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCall {
    pub destination: &'static str,
    pub path: &'static str,
    pub interface: &'static str,
    pub method: &'static str,
}

/// The D-Bus call an overview action stands for, or `None` for every other action. These are
/// the names `cosmic-workspaces` itself serves (`#[zbus::interface(name = "com.system76.
/// CosmicWorkspaces")]` with `show` and `hide` at `/com/system76/CosmicWorkspaces`; zbus
/// capitalizes method names, which `busctl --user call` confirmed on a running session).
pub fn overview_call(action: Action) -> Option<SessionCall> {
    const BUS: &str = "com.system76.CosmicWorkspaces";
    const PATH: &str = "/com/system76/CosmicWorkspaces";
    let method = match action {
        Action::OverviewShow => "Show",
        Action::OverviewHide => "Hide",
        _ => return None,
    };
    Some(SessionCall {
        destination: BUS,
        path: PATH,
        interface: BUS,
        method,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// Axis not locked yet.
    Undecided,
    /// Following the fingers across workspaces. `sign` converts pixels along the axis to
    /// workspace units.
    Workspace { sign: f64 },
    /// Vertical swipe that opens/closes the overview.
    Overview,
    /// Locked, but deliberately does nothing (and is not forwarded).
    Ignored,
}

#[derive(Debug, Clone)]
pub struct Session {
    tuning: Tuning,
    recognizer: Recognizer,
    mode: Mode,
    /// The overview trigger has been crossed in this gesture (it fires at most once).
    fired: bool,
}

impl Session {
    pub fn new(tuning: Tuning) -> Self {
        Session {
            tuning,
            recognizer: Recognizer::new(tuning.recognizer),
            mode: Mode::Undecided,
            fired: false,
        }
    }

    /// Start recognition (fingers down).
    pub fn begin(&mut self, fingers: u32, time: Duration) {
        self.mode = Mode::Undecided;
        self.fired = false;
        self.recognizer.feed(Input::Begin { fingers, time });
    }

    /// Whether workspace tracking has started (the axis locked to the workspace axis).
    pub fn is_tracking_workspaces(&self) -> bool {
        matches!(self.mode, Mode::Workspace { .. })
    }

    /// Abandon the gesture from the outside (a new swipe replaces it, its output vanished).
    /// Cancels workspace tracking if it had started; never activates anything.
    pub fn abort(&mut self) -> Action {
        let action = if self.is_tracking_workspaces() {
            Action::WorkspaceCancel
        } else {
            Action::None
        };
        self.mode = Mode::Undecided;
        self.fired = false;
        self.recognizer.feed(Input::End {
            cancelled: true,
            time: Duration::ZERO,
        });
        action
    }

    pub fn feed(&mut self, input: Input, ctx: &Context) -> Action {
        match self.recognizer.feed(input) {
            Recognized::None => Action::None,
            Recognized::Locked {
                axis,
                distance,
                velocity,
            } => self.lock(axis, distance, velocity, ctx),
            Recognized::Progress {
                axis: _,
                distance,
                velocity,
            } => match self.mode {
                Mode::Workspace { sign } => Action::WorkspaceTrack {
                    first: false,
                    total: self.workspaces(sign, distance),
                    velocity: self.workspace_velocity(sign, velocity),
                },
                Mode::Overview => self.vertical_trigger(distance, ctx),
                Mode::Undecided | Mode::Ignored => Action::None,
            },
            Recognized::Finish { velocity, .. } => {
                let action = match self.mode {
                    Mode::Workspace { sign } => Action::WorkspaceFinish {
                        velocity: self.workspace_velocity(sign, velocity),
                    },
                    _ => Action::None,
                };
                self.finish();
                action
            }
            Recognized::Cancel { .. } => {
                let action = if self.is_tracking_workspaces() {
                    Action::WorkspaceCancel
                } else {
                    Action::None
                };
                self.finish();
                action
            }
        }
    }

    fn finish(&mut self) {
        self.mode = Mode::Undecided;
        self.fired = false;
    }

    fn lock(&mut self, axis: Axis, distance: f64, velocity: f64, ctx: &Context) -> Action {
        let workspace_axis = if ctx.vertical_layout {
            Axis::Vertical
        } else {
            Axis::Horizontal
        };
        if axis == workspace_axis {
            if ctx.overview_open {
                self.mode = Mode::Ignored;
                return Action::None;
            }
            let sign = if ctx.natural_scroll { -1.0 } else { 1.0 };
            self.mode = Mode::Workspace { sign };
            Action::WorkspaceTrack {
                first: true,
                total: self.workspaces(sign, distance),
                velocity: self.workspace_velocity(sign, velocity),
            }
        } else if !ctx.vertical_layout {
            // vertical movement with the usual horizontal layout: the overview
            self.mode = Mode::Overview;
            self.vertical_trigger(distance, ctx)
        } else {
            // horizontal movement with a vertical layout: nothing to do
            self.mode = Mode::Ignored;
            Action::None
        }
    }

    /// Fires at most once per gesture, the first time the distance reaches the trigger in
    /// either direction (the crossing itself counts even when it turns out to be a no-op).
    fn vertical_trigger(&mut self, distance: f64, ctx: &Context) -> Action {
        if self.fired {
            return Action::None;
        }
        let trigger = self.tuning.vertical_trigger;
        if distance <= -trigger {
            self.fired = true;
            if ctx.overview_open {
                Action::None
            } else {
                Action::OverviewShow
            }
        } else if distance >= trigger {
            self.fired = true;
            if ctx.overview_open {
                Action::OverviewHide
            } else {
                Action::None
            }
        } else {
            Action::None
        }
    }

    fn workspaces(&self, sign: f64, pixels: f64) -> f64 {
        sign * pixels_to_workspaces(pixels, self.tuning.swipe_length)
    }

    fn workspace_velocity(&self, sign: f64, pixels_per_second: f64) -> f64 {
        sign * pixel_velocity_to_workspaces(pixels_per_second, self.tuning.swipe_length)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::{
        backend::render::animations::spring::SpringParams,
        input::gestures::RubberBand,
        shell::workspace_transition::{DecideParams, Transition},
    };

    fn ms(t: u64) -> Duration {
        Duration::from_millis(t)
    }

    /// A test tuning with round numbers (not the product defaults).
    fn tuning() -> Tuning {
        Tuning {
            recognizer: RecognizerConfig {
                dead_zone: 10.0,
                axis_ratio: 2.0,
                min_fingers: 3,
                max_fingers: 4,
            },
            swipe_length: 100.0,
            vertical_trigger: 50.0,
            velocity_threshold: 0.5,
            position_threshold: 0.5,
            band_stiffness: 0.5,
            band_limit: 0.25,
        }
    }

    fn ctx() -> Context {
        Context {
            natural_scroll: true,
            vertical_layout: false,
            overview_open: false,
        }
    }

    fn started(fingers: u32) -> Session {
        let mut s = Session::new(tuning());
        s.begin(fingers, ms(0));
        s
    }

    fn upd(s: &mut Session, dx: f64, dy: f64, t: u64, c: &Context) -> Action {
        s.feed(Input::Update { dx, dy, time: ms(t) }, c)
    }

    fn end(s: &mut Session, cancelled: bool, t: u64, c: &Context) -> Action {
        s.feed(
            Input::End {
                cancelled,
                time: ms(t),
            },
            c,
        )
    }

    // =========================================================================================
    // ownership
    // =========================================================================================

    #[test]
    fn three_fingers_are_owned_by_the_compositor() {
        assert_eq!(
            decide_owner(3, false, false, &tuning().recognizer),
            Decision::Compositor { cancel_existing: false }
        );
    }

    #[test]
    fn four_fingers_are_owned_by_the_compositor() {
        assert_eq!(
            decide_owner(4, false, false, &tuning().recognizer),
            Decision::Compositor { cancel_existing: false }
        );
    }

    #[test]
    fn fewer_or_more_fingers_go_to_the_client() {
        for fingers in [0, 1, 2, 5, 6] {
            assert_eq!(
                decide_owner(fingers, false, false, &tuning().recognizer),
                Decision::Client { cancel_existing: false },
                "fingers {fingers}"
            );
        }
    }

    #[test]
    fn a_pointer_grab_sends_the_gesture_to_the_client() {
        for fingers in [3, 4] {
            assert_eq!(
                decide_owner(fingers, true, false, &tuning().recognizer),
                Decision::Client { cancel_existing: false }
            );
        }
    }

    #[test]
    fn an_existing_compositor_session_is_flagged_for_a_safe_cancel() {
        assert_eq!(
            decide_owner(3, false, true, &tuning().recognizer),
            Decision::Compositor { cancel_existing: true }
        );
        // even when the new swipe is not eligible the old one must not be left dangling
        assert_eq!(
            decide_owner(2, false, true, &tuning().recognizer),
            Decision::Client { cancel_existing: true }
        );
        assert_eq!(
            decide_owner(3, true, true, &tuning().recognizer),
            Decision::Client { cancel_existing: true }
        );
    }

    #[test]
    fn owner_begin_installs_the_decided_owner() {
        let mut owner: Owner<u32> = Owner::None;
        assert_eq!(owner.kind(), OwnerKind::None);
        assert!(owner.begin(3, false, &tuning(), 7).is_none());
        assert_eq!(owner.kind(), OwnerKind::Compositor);
        owner.end();
        assert_eq!(owner.kind(), OwnerKind::None);
        assert!(owner.begin(2, false, &tuning(), 7).is_none());
        assert_eq!(owner.kind(), OwnerKind::Client);
        owner.end();
        assert!(owner.begin(3, true, &tuning(), 7).is_none());
        assert_eq!(owner.kind(), OwnerKind::Client);
    }

    #[test]
    fn ownership_never_changes_while_a_gesture_is_running() {
        // compositor-owned: stays compositor-owned through every kind of event and context
        let mut owner: Owner<()> = Owner::None;
        owner.begin(3, false, &tuning(), ());
        for overview_open in [false, true, false] {
            let c = Context { overview_open, ..ctx() };
            if let Owner::Compositor { session, .. } = &mut owner {
                upd(session, 30.0, 0.0, 10, &c);
                upd(session, 0.0, -90.0, 20, &c);
            }
            assert_eq!(owner.kind(), OwnerKind::Compositor);
        }
        // client-owned: stays client-owned (nothing in the pure layer can flip it)
        let mut owner: Owner<()> = Owner::None;
        owner.begin(2, false, &tuning(), ());
        assert_eq!(owner.kind(), OwnerKind::Client);
        assert_eq!(owner.kind(), OwnerKind::Client);
    }

    #[test]
    fn a_new_swipe_aborts_a_session_that_was_still_alive_and_reports_how() {
        let mut owner: Owner<&'static str> = Owner::None;
        owner.begin(3, false, &tuning(), "first output");
        // the first gesture got as far as tracking workspaces
        if let Owner::Compositor { session, .. } = &mut owner {
            assert!(matches!(
                upd(session, -40.0, 0.0, 10, &ctx()),
                Action::WorkspaceTrack { first: true, .. }
            ));
        }
        let previous = owner.begin(3, false, &tuning(), "second output");
        let (extra, action) = previous.expect("the old session must be reported");
        assert_eq!(extra, "first output");
        assert_eq!(action, Action::WorkspaceCancel, "an abort must cancel, never commit");
        assert_eq!(owner.kind(), OwnerKind::Compositor);
    }

    #[test]
    fn a_client_owned_swipe_replaced_by_a_new_one_reports_nothing() {
        let mut owner: Owner<()> = Owner::None;
        owner.begin(2, false, &tuning(), ());
        assert!(owner.begin(3, false, &tuning(), ()).is_none());
        assert_eq!(owner.kind(), OwnerKind::Compositor);
    }

    // =========================================================================================
    // tuning
    // =========================================================================================

    #[test]
    fn tuning_defaults_apply_when_nothing_is_set() {
        assert_eq!(Tuning::from_lookup(|_| None), Tuning::default());
    }

    #[test]
    fn tuning_reads_overrides_and_clamps_nonsense() {
        let t = Tuning::from_lookup(|k| match k {
            "COSMIC_MAC_SWIPE_LENGTH" => Some(450.0),
            "COSMIC_MAC_GESTURE_DEAD_ZONE" => Some(-5.0),
            "COSMIC_MAC_GESTURE_AXIS_RATIO" => Some(0.2),
            "COSMIC_MAC_GESTURE_VERTICAL_TRIGGER" => Some(f64::NAN),
            "COSMIC_MAC_GESTURE_BAND_LIMIT" => Some(0.4),
            _ => None,
        });
        assert_eq!(t.swipe_length, 450.0);
        assert_eq!(t.recognizer.dead_zone, 0.0, "negative dead zone clamps to 0");
        assert_eq!(t.recognizer.axis_ratio, 1.0, "ratio below 1 clamps to 1");
        assert_eq!(t.vertical_trigger, Tuning::default().vertical_trigger, "NaN is ignored");
        assert_eq!(t.band_limit, 0.4);
    }

    // =========================================================================================
    // gesture recognition inside the session
    // =========================================================================================

    #[test]
    fn nothing_happens_inside_the_dead_zone() {
        let mut s = started(3);
        assert_eq!(upd(&mut s, 4.0, 1.0, 10, &ctx()), Action::None);
        assert_eq!(upd(&mut s, 3.0, 0.0, 20, &ctx()), Action::None);
        assert!(!s.is_tracking_workspaces());
    }

    #[test]
    fn horizontal_lock_starts_workspace_tracking_with_first_set_once() {
        let mut s = started(3);
        match upd(&mut s, -30.0, 0.0, 10, &ctx()) {
            Action::WorkspaceTrack { first, total, .. } => {
                assert!(first);
                assert!((total - 0.3).abs() < 1e-12, "30 px of 100 px swipe length = 0.3 workspaces");
            }
            other => panic!("{other:?}"),
        }
        match upd(&mut s, -20.0, 0.0, 20, &ctx()) {
            Action::WorkspaceTrack { first, total, .. } => {
                assert!(!first);
                assert!((total - 0.5).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn diagonal_start_is_rejected_until_one_axis_dominates() {
        let mut s = started(3);
        assert_eq!(upd(&mut s, 20.0, 20.0, 10, &ctx()), Action::None);
        assert_eq!(upd(&mut s, 20.0, 20.0, 20, &ctx()), Action::None);
        assert!(!s.is_tracking_workspaces());
        // finally resolves to horizontal
        assert!(matches!(
            upd(&mut s, -150.0, 0.0, 30, &ctx()),
            Action::WorkspaceTrack { first: true, .. }
        ));
    }

    #[test]
    fn a_diagonal_that_never_resolves_does_nothing_at_all() {
        let mut s = started(3);
        upd(&mut s, 20.0, 20.0, 10, &ctx());
        assert_eq!(end(&mut s, false, 20, &ctx()), Action::None);
        let mut s = started(3);
        upd(&mut s, 20.0, 20.0, 10, &ctx());
        assert_eq!(end(&mut s, true, 20, &ctx()), Action::None);
    }

    #[test]
    fn direction_is_stable_after_the_lock() {
        let mut s = started(3);
        upd(&mut s, -40.0, 0.0, 10, &ctx());
        // a large vertical movement later must not change anything
        match upd(&mut s, -5.0, 300.0, 20, &ctx()) {
            Action::WorkspaceTrack { first, total, .. } => {
                assert!(!first);
                assert!((total - 0.45).abs() < 1e-12, "vertical movement ignored, total {total}");
            }
            other => panic!("{other:?}"),
        }
        assert!(s.is_tracking_workspaces());
    }

    #[test]
    fn reversal_is_not_a_cancellation() {
        let mut s = started(3);
        upd(&mut s, -60.0, 0.0, 10, &ctx()); // 0.6
        let back = upd(&mut s, 20.0, 0.0, 20, &ctx()); // 0.4
        let back2 = upd(&mut s, 20.0, 0.0, 30, &ctx()); // 0.2
        for (action, expected) in [(back, 0.4), (back2, 0.2)] {
            match action {
                Action::WorkspaceTrack { first: false, total, .. } => {
                    assert!((total - expected).abs() < 1e-12)
                }
                other => panic!("reversal must keep tracking, got {other:?}"),
            }
        }
        // and only the release decides
        assert!(matches!(
            end(&mut s, false, 40, &ctx()),
            Action::WorkspaceFinish { .. }
        ));
    }

    #[test]
    fn libinput_cancel_cancels_workspace_tracking() {
        let mut s = started(3);
        upd(&mut s, -60.0, 0.0, 10, &ctx());
        assert_eq!(end(&mut s, true, 20, &ctx()), Action::WorkspaceCancel);
    }

    #[test]
    fn a_normal_release_finishes_with_workspace_units() {
        let mut s = started(3);
        // 10 px every 10 ms leftwards: 1000 px/s -> 10 workspaces/s with a 100 px swipe length
        for i in 1..=6u64 {
            upd(&mut s, -10.0, 0.0, i * 10, &ctx());
        }
        match end(&mut s, false, 70, &ctx()) {
            Action::WorkspaceFinish { velocity } => {
                assert!((velocity - 10.0).abs() < 1e-6, "velocity {velocity}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn velocity_uses_the_swipe_length_and_not_the_output_width() {
        let run = |swipe_length: f64| {
            let mut t = tuning();
            t.swipe_length = swipe_length;
            let mut s = Session::new(t);
            s.begin(3, ms(0));
            for i in 1..=6u64 {
                s.feed(Input::Update { dx: -10.0, dy: 0.0, time: ms(i * 10) }, &ctx());
            }
            match s.feed(Input::End { cancelled: false, time: ms(70) }, &ctx()) {
                Action::WorkspaceFinish { velocity } => velocity,
                other => panic!("{other:?}"),
            }
        };
        let v100 = run(100.0);
        let v200 = run(200.0);
        assert!((v100 / v200 - 2.0).abs() < 1e-9, "halving the swipe length doubles the speed in workspaces/s");
    }

    #[test]
    fn three_and_four_fingers_behave_identically() {
        let run = |fingers: u32| {
            let mut s = started(fingers);
            let mut out = Vec::new();
            for i in 1..=6u64 {
                out.push(upd(&mut s, 12.0, 1.0, i * 10, &ctx()));
            }
            out.push(end(&mut s, false, 70, &ctx()));
            out
        };
        assert_eq!(run(3), run(4));
    }

    // =========================================================================================
    // sign convention (must match the stock mapping)
    // =========================================================================================

    #[test]
    fn natural_scroll_fingers_left_is_next_and_fingers_right_is_previous() {
        let c = Context { natural_scroll: true, ..ctx() };
        let mut left = started(3);
        match upd(&mut left, -40.0, 0.0, 10, &c) {
            Action::WorkspaceTrack { total, .. } => assert!(total > 0.0, "left = toward a higher index"),
            other => panic!("{other:?}"),
        }
        let mut right = started(3);
        match upd(&mut right, 40.0, 0.0, 10, &c) {
            Action::WorkspaceTrack { total, .. } => assert!(total < 0.0, "right = toward a lower index"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn without_natural_scroll_the_mapping_is_reversed() {
        let c = Context { natural_scroll: false, ..ctx() };
        let mut left = started(3);
        match upd(&mut left, -40.0, 0.0, 10, &c) {
            Action::WorkspaceTrack { total, .. } => assert!(total < 0.0),
            other => panic!("{other:?}"),
        }
        let mut right = started(3);
        match upd(&mut right, 40.0, 0.0, 10, &c) {
            Action::WorkspaceTrack { total, .. } => assert!(total > 0.0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn delta_and_velocity_signs_agree_in_both_directions() {
        for natural in [true, false] {
            let c = Context { natural_scroll: natural, ..ctx() };
            for dir in [-1.0f64, 1.0] {
                let mut s = started(3);
                let mut last_total = 0.0;
                for i in 1..=6u64 {
                    if let Action::WorkspaceTrack { total, .. } = upd(&mut s, dir * 10.0, 0.0, i * 10, &c) {
                        last_total = total;
                    }
                }
                match end(&mut s, false, 70, &c) {
                    Action::WorkspaceFinish { velocity } => {
                        assert_eq!(velocity.signum(), last_total.signum(), "natural {natural} dir {dir}");
                    }
                    other => panic!("{other:?}"),
                }
            }
        }
    }

    #[test]
    fn vertical_layout_uses_the_vertical_axis_for_workspaces() {
        let c = Context { vertical_layout: true, ..ctx() };
        // natural scroll: fingers up (negative y) = next
        let mut up = started(3);
        match upd(&mut up, 0.0, -40.0, 10, &c) {
            Action::WorkspaceTrack { first: true, total, .. } => assert!(total > 0.0),
            other => panic!("{other:?}"),
        }
        // horizontal movement does nothing, and neither does it open the overview
        let mut sideways = started(3);
        assert_eq!(upd(&mut sideways, -80.0, 0.0, 10, &c), Action::None);
        assert_eq!(upd(&mut sideways, -80.0, 0.0, 20, &c), Action::None);
    }

    // =========================================================================================
    // overview
    // =========================================================================================

    #[test]
    fn swipe_up_opens_the_overview_when_it_is_closed() {
        let mut s = started(3);
        assert_eq!(upd(&mut s, 0.0, -30.0, 10, &ctx()), Action::None, "below the distance threshold");
        assert_eq!(upd(&mut s, 0.0, -30.0, 20, &ctx()), Action::OverviewShow);
    }

    #[test]
    fn swipe_down_closes_the_overview_when_it_is_open() {
        let c = Context { overview_open: true, ..ctx() };
        let mut s = started(3);
        assert_eq!(upd(&mut s, 0.0, 30.0, 10, &c), Action::None);
        assert_eq!(upd(&mut s, 0.0, 30.0, 20, &c), Action::OverviewHide);
    }

    #[test]
    fn swipe_up_while_the_overview_is_open_is_a_noop() {
        let c = Context { overview_open: true, ..ctx() };
        let mut s = started(3);
        for i in 1..=10u64 {
            assert_eq!(upd(&mut s, 0.0, -20.0, i * 10, &c), Action::None);
        }
        assert_eq!(end(&mut s, false, 120, &c), Action::None);
    }

    #[test]
    fn swipe_down_while_the_overview_is_closed_is_a_noop() {
        let mut s = started(3);
        for i in 1..=10u64 {
            assert_eq!(upd(&mut s, 0.0, 20.0, i * 10, &ctx()), Action::None);
        }
        assert_eq!(end(&mut s, false, 120, &ctx()), Action::None);
    }

    #[test]
    fn horizontal_swipe_while_the_overview_is_open_is_a_noop_and_never_tracks() {
        let c = Context { overview_open: true, ..ctx() };
        let mut s = started(3);
        for i in 1..=10u64 {
            assert_eq!(upd(&mut s, -30.0, 0.0, i * 10, &c), Action::None);
        }
        assert!(!s.is_tracking_workspaces());
        assert_eq!(end(&mut s, false, 120, &c), Action::None);
    }

    #[test]
    fn the_overview_trigger_fires_only_once_per_gesture() {
        let mut s = started(3);
        let mut shows = 0;
        for i in 1..=30u64 {
            if upd(&mut s, 0.0, -20.0, i * 10, &ctx()) == Action::OverviewShow {
                shows += 1;
            }
        }
        assert_eq!(shows, 1);
    }

    #[test]
    fn the_overview_does_not_retrigger_after_a_reversal_in_the_same_gesture() {
        let mut s = started(3);
        let mut actions = Vec::new();
        for i in 1..=5u64 {
            actions.push(upd(&mut s, 0.0, -20.0, i * 10, &ctx())); // up past the trigger
        }
        let open = Context { overview_open: true, ..ctx() };
        for i in 6..=20u64 {
            actions.push(upd(&mut s, 0.0, 20.0, i * 10, &open)); // then back down past +trigger
        }
        assert_eq!(actions.iter().filter(|a| **a == Action::OverviewShow).count(), 1);
        assert_eq!(actions.iter().filter(|a| **a == Action::OverviewHide).count(), 0, "once per gesture");
    }

    #[test]
    fn the_overview_trigger_happens_before_the_fingers_lift() {
        let mut s = started(3);
        let mut fired_at = None;
        for i in 1..=10u64 {
            if upd(&mut s, 0.0, -20.0, i * 10, &ctx()) == Action::OverviewShow {
                fired_at = Some(i);
            }
        }
        assert_eq!(fired_at, Some(3), "fires as soon as 50 px of upward travel (plus lock) is reached");
        assert_eq!(end(&mut s, false, 120, &ctx()), Action::None, "nothing more on release");
    }

    #[test]
    fn the_vertical_trigger_needs_a_locked_vertical_axis() {
        // mostly horizontal with some upward drift never opens the overview
        let mut s = started(3);
        for i in 1..=10u64 {
            let a = upd(&mut s, -30.0, -8.0, i * 10, &ctx());
            assert_ne!(a, Action::OverviewShow);
        }
    }

    #[test]
    fn a_vertical_gesture_never_moves_workspaces() {
        let mut s = started(3);
        for i in 1..=10u64 {
            let a = upd(&mut s, 3.0, -30.0, i * 10, &ctx());
            assert!(!matches!(a, Action::WorkspaceTrack { .. }), "{a:?}");
        }
    }

    #[test]
    fn a_new_gesture_starts_clean_after_an_overview_trigger() {
        let mut s = started(3);
        for i in 1..=10u64 {
            upd(&mut s, 0.0, -20.0, i * 10, &ctx());
        }
        end(&mut s, false, 120, &ctx());
        s.begin(3, ms(200));
        let mut shows = 0;
        for i in 1..=10u64 {
            if upd(&mut s, 0.0, -20.0, 200 + i * 10, &ctx()) == Action::OverviewShow {
                shows += 1;
            }
        }
        assert_eq!(shows, 1);
    }

    // =========================================================================================
    // session + transition: retargeting, boundaries (the same three steps the shell glue runs)
    // =========================================================================================

    fn spring() -> SpringParams {
        SpringParams::new(1.0, 800.0, 0.0001)
    }

    fn decide_params() -> DecideParams {
        DecideParams {
            velocity_threshold: tuning().velocity_threshold,
            position_threshold: tuning().position_threshold,
        }
    }

    fn band() -> RubberBand {
        RubberBand {
            stiffness: tuning().band_stiffness,
            limit: tuning().band_limit,
        }
    }

    /// The glue the shell runs for each action, on a pure `Transition` plus a logical `active`,
    /// with one clock shared by the gesture's timestamps and the transition.
    struct Shell {
        base: Instant,
        clock: u64,
        active: usize,
        count: usize,
        transition: Transition,
        session: Option<Session>,
    }

    impl Shell {
        fn new(active: usize, count: usize) -> Self {
            Shell {
                base: Instant::now(),
                clock: 0,
                active,
                count,
                transition: Transition::new(active),
                session: None,
            }
        }

        fn last(&self) -> usize {
            self.count - 1
        }

        fn now(&self) -> Instant {
            self.base + ms(self.clock)
        }

        fn wait(&mut self, millis: u64) {
            self.clock += millis;
        }

        fn position(&self) -> f64 {
            self.transition.position(self.now())
        }

        fn velocity(&self) -> f64 {
            self.transition.velocity(self.now())
        }

        fn begin(&mut self, fingers: u32) {
            let mut s = Session::new(tuning());
            s.begin(fingers, ms(self.clock));
            self.session = Some(s);
        }

        fn apply(&mut self, action: Action) {
            let now = self.now();
            match action {
                Action::WorkspaceTrack { first, total, velocity } => {
                    if first {
                        self.transition.begin_tracking(now, self.active);
                    }
                    let last = self.last();
                    self.transition.track(total, velocity, last, &band());
                }
                Action::WorkspaceFinish { velocity } => {
                    if self.transition.is_tracking() {
                        let last = self.last();
                        let target =
                            self.transition.finish(now, self.active, velocity, last, decide_params(), spring());
                        self.active = target;
                    }
                }
                Action::WorkspaceCancel => {
                    if self.transition.is_tracking() {
                        self.transition.cancel(now, self.active, spring());
                    }
                }
                _ => {}
            }
        }

        /// `steps` events of `dx` pixels each, 10 ms apart.
        fn drag(&mut self, dx: f64, steps: u64) {
            for _ in 0..steps {
                self.clock += 10;
                let action = self
                    .session
                    .as_mut()
                    .unwrap()
                    .feed(Input::Update { dx, dy: 0.0, time: ms(self.clock) }, &ctx());
                self.apply(action);
            }
        }

        /// Fingers lift 5 ms after the last event.
        fn release(&mut self, cancelled: bool) {
            self.clock += 5;
            let action = self
                .session
                .as_mut()
                .unwrap()
                .feed(Input::End { cancelled, time: ms(self.clock) }, &ctx());
            self.apply(action);
            self.session = None;
        }

        /// Let everything settle and return the final position.
        fn settle(&mut self) -> f64 {
            self.clock += 20_000;
            let now = self.now();
            self.transition.refresh(now);
            assert!(self.transition.is_idle(), "the transition must come to rest");
            self.transition.position(now)
        }
    }

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn a_swipe_left_commits_to_the_next_workspace_and_settles() {
        let mut sh = Shell::new(1, 4);
        sh.begin(3);
        sh.drag(-10.0, 8); // 80 px = 0.8 workspaces
        assert_eq!(sh.active, 1, "active does not change while dragging");
        assert!(near(sh.position(), 1.8, 1e-9));
        sh.release(false);
        assert_eq!(sh.active, 2);
        assert!(near(sh.settle(), 2.0, 1e-9));
    }

    #[test]
    fn a_swipe_right_commits_to_the_previous_workspace() {
        let mut sh = Shell::new(2, 4);
        sh.begin(4);
        sh.drag(10.0, 8);
        sh.release(false);
        assert_eq!(sh.active, 1);
        assert!(near(sh.settle(), 1.0, 1e-9));
    }

    #[test]
    fn a_cancelled_gesture_never_commits_and_returns_to_the_logical_workspace() {
        let mut sh = Shell::new(1, 4);
        sh.begin(3);
        sh.drag(-12.0, 8); // far enough and fast enough to commit
        sh.release(true);
        assert_eq!(sh.active, 1);
        assert!(near(sh.settle(), 1.0, 1e-9));
    }

    #[test]
    fn pausing_before_lifting_the_fingers_does_not_count_as_a_flick() {
        let mut sh = Shell::new(1, 4);
        sh.begin(3);
        sh.drag(-1.5, 20); // 30 px = 0.3 workspaces, 150 px/s while moving
        sh.wait(400); // fingers rest, then lift
        sh.release(false);
        assert_eq!(sh.active, 1, "0.3 workspaces and no speed at lift-off stays");
        assert!(near(sh.settle(), 1.0, 1e-9));
    }

    #[test]
    fn dragging_back_through_the_origin_works_like_a_single_gesture() {
        let mut sh = Shell::new(1, 4);
        sh.begin(3);
        sh.drag(-10.0, 6); // 0.6 forward
        sh.drag(10.0, 12); // then 1.2 back: 0.6 past the origin on the other side
        assert!(near(sh.position(), 0.4, 1e-9));
        sh.release(false);
        assert_eq!(sh.active, 0, "released while moving back, ends on the lower workspace");
    }

    // ---- retargeting ----------------------------------------------------------------------

    #[test]
    fn a_gesture_starting_during_a_spring_begins_from_the_visual_position_and_velocity() {
        let mut sh = Shell::new(0, 4);
        sh.begin(3);
        sh.drag(-12.0, 5); // A -> B committed
        sh.release(false);
        assert_eq!(sh.active, 1);
        sh.wait(40); // the spring is still on its way
        let p = sh.position();
        let v = sh.velocity();
        assert!(p > 0.0 && p < 1.0 && v > 0.0, "p {p} v {v}");
        sh.begin(3);
        sh.drag(-30.0, 1); // 30 px = 0.3 workspaces in the first event (lock + track)
        assert!(sh.transition.is_tracking());
        // tracking began where the spring was (one 10 ms event later at most) and is not at A
        // or at the logical active workspace B: position = anchor + 0.3 with anchor near `p`
        let anchor = sh.position() - 0.3;
        assert!(
            anchor > p - 0.02 && anchor < 1.0,
            "anchor {anchor} must be where the spring was ({p}), not 0 or 1"
        );
    }

    #[test]
    fn begin_tracking_while_a_spring_runs_keeps_position_and_velocity_exactly() {
        let mut sh = Shell::new(0, 4);
        sh.begin(3);
        sh.drag(-12.0, 5);
        sh.release(false);
        sh.wait(45);
        let p = sh.position();
        let v = sh.velocity();
        sh.transition.begin_tracking(sh.now(), sh.active);
        assert!(near(sh.position(), p, 1e-12), "position continuity");
        assert!(near(sh.velocity(), v, 1e-12), "velocity continuity");
    }

    #[test]
    fn a_gesture_starting_during_a_timed_animation_is_continuous_too() {
        let now = Instant::now();
        let mut t = Transition::new(0);
        t.retarget_timed(now, 1, Duration::from_millis(200)); // keyboard switch
        let mid = now + ms(70);
        let p = t.position(mid);
        let v = t.velocity(mid);
        assert!(v > 0.0);
        t.begin_tracking(mid, 1);
        assert!(near(t.position(mid), p, 1e-12));
        assert!(near(t.velocity(mid), v, 1e-12));
    }

    #[test]
    fn reversing_during_an_animation_returns_smoothly() {
        let mut sh = Shell::new(0, 4);
        sh.begin(3);
        sh.drag(-12.0, 5);
        sh.release(false); // A -> B
        sh.wait(40);
        let p = sh.position();
        // immediately swipe back with a flick
        sh.begin(3);
        let mut last = p;
        for _ in 0..6 {
            sh.drag(12.0, 1);
            let pos = sh.position();
            assert!(pos < last + 1e-9, "moving back toward A: {pos} after {last}");
            last = pos;
        }
        sh.release(false);
        assert_eq!(sh.active, 0, "the flick back commits to A");
        assert!(near(sh.settle(), 0.0, 1e-9));
    }

    #[test]
    fn a_to_b_to_c_without_jumps() {
        let mut sh = Shell::new(0, 4);
        // A -> B
        sh.begin(3);
        sh.drag(-12.0, 5);
        sh.release(false);
        assert_eq!(sh.active, 1);
        // 60 ms later, still on its way, a second flick toward C
        sh.wait(60);
        let p_before = sh.position();
        sh.begin(3);
        sh.drag(-12.0, 6);
        assert!(sh.position() > p_before, "no jump back to A or B");
        sh.release(false);
        assert_eq!(sh.active, 2);
        // follow the settle in 1 ms steps: no sample-to-sample jump
        let mut last = sh.position();
        for _ in 0..800 {
            sh.wait(1);
            let p = sh.position();
            assert!((p - last).abs() < 0.05, "jump of {}", (p - last).abs());
            last = p;
        }
        assert!(near(sh.settle(), 2.0, 1e-9));
    }

    #[test]
    fn rapid_gestures_keep_state_consistent() {
        let mut sh = Shell::new(1, 5);
        let mut seed: u64 = 0x2545F4914F6CDD1D;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let dir = if next() % 2 == 0 { -1.0 } else { 1.0 };
            let steps = 2 + next() % 9;
            let cancelled = next() % 7 == 0;
            sh.wait(next() % 120);
            let before = sh.position();
            sh.begin(3 + (next() % 2) as u32);
            sh.drag(dir * 12.0, 1);
            // the first event of a new gesture starts from where the screen was (no jump)
            // 0.12 workspaces of finger movement plus at most ~10 ms of the running animation;
            // a snap back to the logical workspace would be a jump of up to a whole workspace
            assert!(
                (sh.position() - before).abs() < 0.35,
                "jump at the start of a gesture: {before} -> {}",
                sh.position()
            );
            sh.drag(dir * 12.0, steps - 1);
            sh.release(cancelled);
            assert!(sh.active <= sh.last(), "the logical workspace is always valid");
        }
        let settled = sh.settle();
        assert!(near(settled, sh.active as f64, 1e-9));
    }

    // ---- boundaries -----------------------------------------------------------------------

    #[test]
    fn first_workspace_swipe_toward_previous_is_resisted_and_stays_first() {
        let mut sh = Shell::new(0, 4);
        sh.begin(3);
        sh.drag(12.0, 6); // fingers right = toward a lower index, past the first
        let p = sh.position();
        assert!(p < 0.0 && p > -0.25, "resisted overshoot {p}");
        assert!(p > -0.72, "less than the raw 0.72");
        sh.release(false);
        assert_eq!(sh.active, 0);
        assert!(near(sh.settle(), 0.0, 1e-9));
    }

    #[test]
    fn last_workspace_swipe_toward_next_is_resisted_and_stays_last() {
        let mut sh = Shell::new(3, 4);
        sh.begin(3);
        sh.drag(-12.0, 6);
        let p = sh.position();
        assert!(p > 3.0 && p < 3.25, "p {p}");
        sh.release(false);
        assert_eq!(sh.active, 3);
        assert!(near(sh.settle(), 3.0, 1e-9));
    }

    #[test]
    fn single_workspace_resists_both_directions_and_never_leaves_it() {
        for dx in [-12.0, 12.0] {
            let mut sh = Shell::new(0, 1);
            sh.begin(3);
            sh.drag(dx, 6);
            assert!(sh.position().abs() < 0.25);
            sh.release(false);
            assert_eq!(sh.active, 0);
            assert!(near(sh.settle(), 0.0, 1e-9));
        }
    }

    #[test]
    fn a_huge_overshoot_stays_bounded() {
        let mut sh = Shell::new(0, 3);
        sh.begin(3);
        sh.drag(50.0, 200); // 100 workspaces worth of travel toward "previous"
        let p = sh.position();
        assert!(p < 0.0 && p > -tuning().band_limit, "p {p}");
        sh.release(false);
        assert_eq!(sh.active, 0);
        assert!(near(sh.settle(), 0.0, 1e-9));
    }

    #[test]
    fn release_at_the_edge_fast_does_not_activate_a_missing_workspace() {
        let mut sh = Shell::new(2, 3);
        sh.begin(3);
        sh.drag(-30.0, 10); // very fast toward "next" with no next
        sh.release(false);
        assert_eq!(sh.active, 2);
        assert!(near(sh.settle(), 2.0, 1e-9));
    }

    #[test]
    fn cancel_at_the_edge_returns_to_the_edge_workspace() {
        let mut sh = Shell::new(0, 3);
        sh.begin(3);
        sh.drag(12.0, 6);
        sh.release(true);
        assert_eq!(sh.active, 0);
        assert!(near(sh.settle(), 0.0, 1e-9));
    }

    #[test]
    fn an_external_activation_during_a_drag_wins_and_the_late_release_is_harmless() {
        let mut sh = Shell::new(0, 4);
        sh.begin(3);
        sh.drag(-10.0, 4);
        // a keyboard shortcut takes over: the transition is no longer tracking
        sh.active = 2;
        let now = sh.now();
        sh.transition.retarget_timed(now, 2, Duration::from_millis(200));
        // the gesture keeps going and ends: nothing must change
        sh.drag(-10.0, 3);
        sh.release(false);
        assert_eq!(sh.active, 2);
        assert!(near(sh.settle(), 2.0, 1e-9));
    }

    // =========================================================================================
    // A4: overview calls
    // =========================================================================================

    #[test]
    fn show_and_hide_map_to_the_cosmic_workspaces_methods() {
        let show = overview_call(Action::OverviewShow).unwrap();
        let hide = overview_call(Action::OverviewHide).unwrap();
        for call in [show, hide] {
            assert_eq!(call.destination, "com.system76.CosmicWorkspaces");
            assert_eq!(call.path, "/com/system76/CosmicWorkspaces");
            assert_eq!(call.interface, "com.system76.CosmicWorkspaces");
        }
        assert_eq!(show.method, "Show");
        assert_eq!(hide.method, "Hide");
    }

    #[test]
    fn every_other_action_makes_no_d_bus_call() {
        for action in [
            Action::None,
            Action::WorkspaceTrack { first: true, total: 0.3, velocity: 1.0 },
            Action::WorkspaceTrack { first: false, total: -0.3, velocity: -1.0 },
            Action::WorkspaceFinish { velocity: 2.0 },
            Action::WorkspaceCancel,
        ] {
            assert_eq!(overview_call(action), None, "{action:?}");
        }
    }

    #[test]
    fn an_overview_gesture_only_ever_produces_overview_actions_or_none() {
        // whatever the overview state does during the gesture, no workspace action appears, so
        // nothing the D-Bus side does (or fails to do) can reach the workspace state
        for start_open in [false, true] {
            for dy in [-20.0f64, 20.0] {
                let mut s = started(3);
                for i in 1..=15u64 {
                    // the overview flips open/closed part-way, as it would after a call
                    let open = if i > 5 { !start_open } else { start_open };
                    let c = Context { overview_open: open, ..ctx() };
                    let a = upd(&mut s, 2.0, dy, i * 10, &c);
                    assert!(
                        matches!(a, Action::None | Action::OverviewShow | Action::OverviewHide),
                        "start_open {start_open} dy {dy}: {a:?}"
                    );
                }
                let a = end(&mut s, false, 200, &ctx());
                assert_eq!(a, Action::None);
            }
        }
    }

    #[test]
    fn a_failed_overview_call_leaves_the_gesture_and_workspaces_consistent() {
        // The glue ignores the outcome of the D-Bus call: model a failed call as "the overview
        // never opens". The gesture keeps going, ends cleanly, the next gesture works, and the
        // workspace state never moved.
        let mut sh = Shell::new(2, 4);
        let before_active = sh.active;
        let mut shows = 0;
        for _ in 0..3 {
            sh.begin(3);
            for _ in 0..8 {
                sh.clock += 10;
                let action = sh.session.as_mut().unwrap().feed(
                    Input::Update { dx: 0.0, dy: -20.0, time: ms(sh.clock) },
                    &ctx(), // the overview stays closed: the call "failed"
                );
                if action == Action::OverviewShow {
                    shows += 1;
                    assert!(overview_call(action).is_some());
                }
                sh.apply(action);
            }
            sh.release(false);
            sh.wait(100);
        }
        assert_eq!(shows, 3, "one attempt per gesture, none repeated within a gesture");
        assert_eq!(sh.active, before_active);
        assert!(sh.transition.is_idle());
        assert!(near(sh.position(), before_active as f64, 1e-12));
    }

    #[test]
    fn horizontal_swipe_while_the_overview_is_open_never_touches_the_transition() {
        let mut sh = Shell::new(1, 4);
        let open = Context { overview_open: true, ..ctx() };
        sh.begin(4);
        for _ in 0..10 {
            sh.clock += 10;
            let action = sh.session.as_mut().unwrap().feed(
                Input::Update { dx: -15.0, dy: 1.0, time: ms(sh.clock) },
                &open,
            );
            assert_eq!(action, Action::None);
            sh.apply(action);
        }
        assert!(sh.transition.is_idle(), "no tracking may start");
        assert_eq!(sh.active, 1);
    }
}
