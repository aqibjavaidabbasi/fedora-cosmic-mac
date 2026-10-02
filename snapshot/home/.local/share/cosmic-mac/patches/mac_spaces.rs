// SPDX-License-Identifier: GPL-3.0-only
//
// cosmic-mac Stage B1: the Mac "Spaces" policy, described as pure decisions.
//
// PURE: no smithay, Wayland, Shell, protocol, timer, global or async state. Nothing in the
// compositor calls this module (B1 is behavior-neutral), so everything here is dead code in the
// running compositor. Its job is to make the policy explicit and testable before any stage moves
// it out of the `cosmic-maximize-to-workspace` helper.
#![allow(dead_code)]

//! # What this module describes
//!
//! Today the Spaces policy lives in `cosmic-maximize-to-workspace`, a Wayland client that reacts
//! after the compositor has already changed state. This module writes the rules that helper
//! applies as pure functions of a per-output snapshot, so each rule is a named, tested decision:
//!
//! | # | Rule | Function |
//! |---|------|----------|
//! | 1 | Desktop selection | [`select_desktop`], [`current_helper_desktop_to_pin`] |
//! | 2 | A window becomes maximized/fullscreen | [`classify_change`], [`on_became_big`] |
//! | 3 | A window stops being big (restored, or minimized) | [`on_stopped_being_big`] |
//! | 4 | The owner of a dedicated Space is closed | [`on_owner_closed`] |
//! | 5 | Which workspaces may be pruned | [`current_prune_verdicts`] |
//! | 6 | How an existing dedicated Space is identified | [`classify_workspace`] |
//! | 7 | A window is moved to another workspace (explicit user moves are their own case) | [`on_window_moved`] |
//! | – | Two desktops on one output (B6, deliberately undecided) | [`multiple_desktops`] |
//! | – | Desired Spaces invariants, checked rather than assumed | [`spaces_violations`] |
//!
//! Every test name starts with one of three labels:
//! - `current_helper_` / `current_compositor_`: what the running system does today;
//! - `intended_`: the native Spaces direction that has been agreed;
//! - `undecided_`: behavior that is still an open Stage B decision. The test pins today's
//!   behavior so a later change is visible, it does **not** endorse it.
//!
//! # The snapshot model
//! [`OutputSnapshot`] mirrors one compositor `WorkspaceSet` (one per output in `OutputBound`
//! mode): the ordered workspaces of that output and the index of its active one. Each
//! [`WorkspaceInfo`] mirrors a compositor `Workspace`: `pinned`, the windows mapped on it
//! (floating, tiling, minimized and fullscreen, the same set `Workspace::is_empty` looks at),
//! whether it holds a live activation token, and whether it was created recently.
//!
//! The model is deliberately permissive. It can hold zero desktops, several desktops, or a
//! maximized window sharing a workspace with others, because the real COSMIC workspace model can
//! hold all of those. "One desktop per output" and "a big window never shares its workspace" are
//! *desired policy outcomes*, reported by [`spaces_violations`], never assumed by the functions.
//!
//! Not modelled (on purpose, and documented here so it is not a hidden assumption):
//! - sticky windows: the compositor keeps them in the output's `sticky_layer`, not in any
//!   workspace, and the helper ignores them;
//! - which output a decision applies to: every function takes one output's snapshot, so a
//!   decision such as [`BigDecision::CreateSpace`] means "on this output";
//! - how a decision is carried out (protocol requests, round trips, animations). That is the
//!   B3 mechanics question and does not belong in a policy description;
//! - time. "Recently created" is an input; the 3 s windows used today live in the compositor
//!   (`MAC_RECENT`) and the helper (`pending_new`), not here.

/// Opaque window identity (a toplevel). Only compared for equality.
pub type WindowId = u64;

/// The parts of a window's state the policy looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowState {
    pub maximized: bool,
    pub fullscreen: bool,
    pub minimized: bool,
}

impl WindowState {
    /// "Big" as the helper defines it (`is_big`): maximized or fullscreen, and not minimized.
    /// (The helper also excludes sticky windows; those are not in a workspace, see the module
    /// docs.)
    pub fn is_big(&self) -> bool {
        (self.maximized || self.fullscreen) && !self.minimized
    }

    /// Shown on its workspace (the helper's `visible_toplevels_on`).
    pub fn is_visible(&self) -> bool {
        !self.minimized
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub state: WindowState,
}

/// One compositor `Workspace`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceInfo {
    /// `Workspace::pinned` (the zcosmic-workspace-v2 Pinned state, persisted by the compositor).
    pub pinned: bool,
    /// Every window mapped on this workspace, minimized and fullscreen ones included.
    pub windows: Vec<WindowInfo>,
    /// A live xdg-activation token targets this workspace (`Workspace::has_activation_token`).
    pub has_activation_token: bool,
    /// Created within the compositor's "recent" window (today `MAC_RECENT`, 3 s).
    pub recently_created: bool,
}

impl WorkspaceInfo {
    /// `Workspace::is_empty`: no windows at all, minimized and fullscreen included.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    fn contains(&self, window: WindowId) -> bool {
        self.windows.iter().any(|w| w.id == window)
    }

    fn visible(&self) -> impl Iterator<Item = &WindowInfo> {
        self.windows.iter().filter(|w| w.state.is_visible())
    }

    /// Visible windows other than `except` (the helper's `others_on`).
    fn visible_others(&self, except: WindowId) -> usize {
        self.visible().filter(|w| w.id != except).count()
    }
}

/// One output's workspace set, in order (index 0 is leftmost in a horizontal layout).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputSnapshot {
    pub workspaces: Vec<WorkspaceInfo>,
    /// `WorkspaceSet::active`. May be out of range in a malformed snapshot; every function
    /// treats that as "no active workspace" instead of panicking.
    pub active: usize,
}

impl OutputSnapshot {
    fn workspace(&self, idx: usize) -> Option<&WorkspaceInfo> {
        self.workspaces.get(idx)
    }
}

// ---------------------------------------------------------------------------------------------
// 1. desktop selection
// ---------------------------------------------------------------------------------------------

/// Which workspace of an output is its desktop, as the helper sees it (`desktop_of`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopSelection {
    /// Exactly one pinned workspace: that is the desktop, wherever it sits in the row.
    Pinned(usize),
    /// No pinned workspace yet. The helper falls back to the leftmost workspace until it pins one
    /// (see [`current_helper_desktop_to_pin`]).
    Unpinned { fallback: usize },
    /// Several pinned workspaces on one output. The helper uses the first; what the native
    /// policy should do is the B6 question (see [`multiple_desktops`]).
    Conflict { first: usize, others: Vec<usize> },
    /// The output has no workspaces (the compositor never leaves a set empty for long).
    NoWorkspaces,
}

impl DesktopSelection {
    /// The index the helper treats as the desktop.
    pub fn index(&self) -> Option<usize> {
        match self {
            DesktopSelection::Pinned(i) => Some(*i),
            DesktopSelection::Unpinned { fallback } => Some(*fallback),
            DesktopSelection::Conflict { first, .. } => Some(*first),
            DesktopSelection::NoWorkspaces => None,
        }
    }
}

/// Rule 1: the desktop is recognised by being pinned, not by its position.
pub fn select_desktop(snap: &OutputSnapshot) -> DesktopSelection {
    let pinned: Vec<usize> = snap
        .workspaces
        .iter()
        .enumerate()
        .filter(|(_, w)| w.pinned)
        .map(|(i, _)| i)
        .collect();
    match pinned.as_slice() {
        [] if snap.workspaces.is_empty() => DesktopSelection::NoWorkspaces,
        [] => DesktopSelection::Unpinned { fallback: 0 },
        [one] => DesktopSelection::Pinned(*one),
        [first, rest @ ..] => DesktopSelection::Conflict {
            first: *first,
            others: rest.to_vec(),
        },
    }
}

/// Current helper (`enforce_desktops`): an output without a pinned workspace gets its first
/// workspace that shows no big window pinned. `None` if one is already pinned or every
/// workspace shows a big window.
pub fn current_helper_desktop_to_pin(snap: &OutputSnapshot) -> Option<usize> {
    if snap.workspaces.iter().any(|w| w.pinned) {
        return None;
    }
    snap.workspaces
        .iter()
        .position(|w| !w.visible().any(|t| t.state.is_big()))
}

// ---------------------------------------------------------------------------------------------
// 6. identifying a dedicated Space
// ---------------------------------------------------------------------------------------------

/// What a workspace is, under the helper's rules. Today there is no explicit "Space" marker in
/// the compositor; a Space is *inferred* from its contents. (Whether native Spaces get an
/// explicit owner marker is a B3 decision and is not modelled here.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceKind {
    /// The output's desktop (see [`select_desktop`]).
    Desktop,
    /// Not the desktop, and its only visible window is big: that window's dedicated Space.
    DedicatedSpace { owner: WindowId },
    /// Not the desktop, holds a big window and other visible windows. The policy wants to
    /// split these; the helper only does so when one of them changes state or moves.
    SharedWithBig { big: Vec<WindowId> },
    /// Not the desktop, no window at all.
    Empty,
    /// Not the desktop, holds windows but no visible big one (e.g. only minimized windows,
    /// or normal windows on an extra workspace).
    Ordinary,
}

/// Rule 6. `None` if `idx` is out of range.
pub fn classify_workspace(snap: &OutputSnapshot, idx: usize) -> Option<WorkspaceKind> {
    let ws = snap.workspace(idx)?;
    if select_desktop(snap).index() == Some(idx) {
        return Some(WorkspaceKind::Desktop);
    }
    if ws.is_empty() {
        return Some(WorkspaceKind::Empty);
    }
    let visible: Vec<&WindowInfo> = ws.visible().collect();
    let big: Vec<WindowId> = visible
        .iter()
        .filter(|w| w.state.is_big())
        .map(|w| w.id)
        .collect();
    Some(match (big.as_slice(), visible.len()) {
        ([], _) => WorkspaceKind::Ordinary,
        ([owner], 1) => WorkspaceKind::DedicatedSpace { owner: *owner },
        _ => WorkspaceKind::SharedWithBig { big },
    })
}

// ---------------------------------------------------------------------------------------------
// 2./3. state changes
// ---------------------------------------------------------------------------------------------

/// How a window's big-ness changed between two states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    BecameBig(BigCause),
    StoppedBeingBig(SmallCause),
    /// Big-ness did not change (it may still have moved: see [`on_window_moved`]).
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigCause {
    /// Maximized or fullscreened.
    Enlarged,
    /// A maximized/fullscreen window was un-minimized. The helper cannot tell this apart from
    /// maximizing.
    Unminimized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmallCause {
    /// Unmaximized / left fullscreen.
    Restored,
    /// Minimized while big. The helper cannot tell this apart from restoring.
    Minimized,
}

/// The helper only sees "big" flip; this keeps the cause so the cases can be told apart.
pub fn classify_change(before: WindowState, after: WindowState) -> StateChange {
    match (before.is_big(), after.is_big()) {
        (false, true) if before.minimized && !after.minimized => {
            StateChange::BecameBig(BigCause::Unminimized)
        }
        (false, true) => StateChange::BecameBig(BigCause::Enlarged),
        (true, false) if after.minimized => StateChange::StoppedBeingBig(SmallCause::Minimized),
        (true, false) => StateChange::StoppedBeingBig(SmallCause::Restored),
        _ => StateChange::Unchanged,
    }
}

/// What to do with a window that just became big (rule 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigDecision {
    /// It is already alone on a workspace that is not the desktop: leave it there.
    AlreadyOwnSpace,
    /// Move it to this existing empty workspace and show that workspace (helper behavior in
    /// stock COSMIC, where an empty workspace is kept at the end; whether native Spaces ever
    /// reuse a workspace is a B3 decision).
    ReuseEmptyWorkspace(usize),
    /// Give it a new workspace at the end of this output's row and show it.
    CreateSpace,
}

/// Rule 2, as the helper applies it (`handle` + `give_own_workspace`). `ws` is the workspace
/// the window is on. `None` if `ws` is out of range or the window is not on it.
pub fn on_became_big(snap: &OutputSnapshot, ws: usize, window: WindowId) -> Option<BigDecision> {
    let here = snap.workspace(ws)?;
    if !here.contains(window) {
        return None;
    }
    let desktop = select_desktop(snap).index();
    if desktop != Some(ws) && here.visible_others(window) == 0 {
        return Some(BigDecision::AlreadyOwnSpace);
    }
    let last = snap.workspaces.len() - 1;
    let last_ws = &snap.workspaces[last];
    if last != ws && desktop != Some(last) && last_ws.visible().next().is_none() {
        return Some(BigDecision::ReuseEmptyWorkspace(last));
    }
    Some(BigDecision::CreateSpace)
}

/// What to do with a window that stopped being big (rule 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreDecision {
    /// Already on the desktop: nothing to move.
    StayOnDesktop,
    /// Move it to the desktop and show the desktop.
    MoveToDesktop { desktop: usize },
    /// The output has no desktop to go to.
    NoDesktop,
}

/// Rule 3, as the helper applies it. The helper applies the same rule whether the window was
/// restored or minimized (`_cause` is accepted so callers and tests state which case they
/// mean; see `undecided_minimizing_a_big_window_*`). `None` if `ws` is out of range or the
/// window is not on it.
pub fn on_stopped_being_big(
    snap: &OutputSnapshot,
    ws: usize,
    window: WindowId,
    _cause: SmallCause,
) -> Option<RestoreDecision> {
    if !snap.workspace(ws)?.contains(window) {
        return None;
    }
    Some(match select_desktop(snap).index() {
        None => RestoreDecision::NoDesktop,
        Some(d) if d == ws => RestoreDecision::StayOnDesktop,
        Some(d) => RestoreDecision::MoveToDesktop { desktop: d },
    })
}

// ---------------------------------------------------------------------------------------------
// 4. closing a Space's owner
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Show the desktop. The emptied Space is then left to pruning (rule 5).
    ActivateDesktop(usize),
    /// Do nothing (the reason says why).
    Nothing(CloseNothingReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseNothingReason {
    WasNotBig,
    WorkspaceNotActive,
    OtherWindowsRemain,
    AlreadyOnDesktop,
    NoDesktop,
}

/// Rule 4 (helper `toplevel_closed`). `snap` is the state *before* the window is removed, as the
/// helper sees it in the closed event; `was_big` is the last big-ness it recorded. `None` if
/// `ws` is out of range or the window is not on it.
pub fn on_owner_closed(
    snap: &OutputSnapshot,
    ws: usize,
    window: WindowId,
    was_big: bool,
) -> Option<CloseDecision> {
    let here = snap.workspace(ws)?;
    if !here.contains(window) {
        return None;
    }
    use CloseNothingReason::*;
    Some(if !was_big {
        CloseDecision::Nothing(WasNotBig)
    } else if snap.active != ws {
        CloseDecision::Nothing(WorkspaceNotActive)
    } else if here.visible_others(window) > 0 {
        CloseDecision::Nothing(OtherWindowsRemain)
    } else {
        match select_desktop(snap).index() {
            None => CloseDecision::Nothing(NoDesktop),
            Some(d) if d == ws => CloseDecision::Nothing(AlreadyOnDesktop),
            Some(d) => CloseDecision::ActivateDesktop(d),
        }
    })
}

// ---------------------------------------------------------------------------------------------
// 5. pruning
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneVerdict {
    Keep(KeepReason),
    Remove,
}

/// Why a workspace survives pruning, in the order the compositor checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    /// `mac_prune` always keeps index 0, whether or not it is the desktop.
    IndexZero,
    Active,
    RecentlyCreated,
    NotEmpty,
    ActivationToken,
    Pinned,
}

/// Rule 5, exactly as the compositor's `mac_prune` decides today (`COSMIC_MAC_SPACES=1`):
/// keep index 0, the active workspace, recently created ones, and anything
/// `can_auto_remove` refuses (not empty, activation token, pinned). An empty snapshot gives an
/// empty list (the compositor then adds one workspace; that is mechanics, not policy).
pub fn current_prune_verdicts(snap: &OutputSnapshot) -> Vec<PruneVerdict> {
    snap.workspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            use KeepReason::*;
            let reason = if i == 0 {
                Some(IndexZero)
            } else if i == snap.active {
                Some(Active)
            } else if w.recently_created {
                Some(RecentlyCreated)
            } else if !w.is_empty() {
                Some(NotEmpty)
            } else if w.has_activation_token {
                Some(ActivationToken)
            } else if w.pinned {
                Some(Pinned)
            } else {
                None
            };
            reason.map_or(PruneVerdict::Remove, PruneVerdict::Keep)
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// 7. moves
// ---------------------------------------------------------------------------------------------

/// Who moved a window. The helper cannot observe this today: everything it sees is `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOrigin {
    /// The user moved it on purpose (shortcut, drag, overview).
    User,
    /// The Spaces policy itself moved it (the result of one of the decisions above).
    Policy,
    /// Not known (everything the helper sees today).
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDecision {
    /// Leave the window where it was put.
    Respect,
    /// Send it to the output's desktop and show the desktop (helper: a normal window that lands
    /// in a big window's Space).
    SendToDesktop(usize),
    /// Give it a Space of its own (helper: a big window that lands on the desktop or a shared
    /// workspace). The inner decision is [`on_became_big`] at the destination.
    GiveOwnSpace(BigDecision),
}

/// Rule 7. `to` is the workspace the window is on after the move.
///
/// - `User`: **respected** (agreed B5 direction: an explicit user move is not bounced back by
///   the Spaces policy). Recorded here; B5 has not wired it into anything.
/// - `Policy`: respected (the policy does not second-guess its own move).
/// - `Unknown`: the current helper rules (`handle_moved`), which bounce.
///
/// `None` if `to` is out of range or the window is not on it.
pub fn on_window_moved(
    snap: &OutputSnapshot,
    to: usize,
    window: WindowId,
    origin: MoveOrigin,
) -> Option<MoveDecision> {
    let here = snap.workspace(to)?;
    let info = here.windows.iter().find(|w| w.id == window)?;
    match origin {
        MoveOrigin::User | MoveOrigin::Policy => return Some(MoveDecision::Respect),
        MoveOrigin::Unknown => {}
    }
    let desktop = select_desktop(snap).index();
    if !info.state.is_big() {
        let holds_big = here.visible().any(|w| w.id != window && w.state.is_big());
        return Some(match desktop {
            Some(d) if d != to && holds_big => MoveDecision::SendToDesktop(d),
            _ => MoveDecision::Respect,
        });
    }
    if desktop == Some(to) || here.visible_others(window) > 0 {
        return on_became_big(snap, to, window).map(MoveDecision::GiveOwnSpace);
    }
    Some(MoveDecision::Respect)
}

// ---------------------------------------------------------------------------------------------
// B6 (undecided): several desktops on one output
// ---------------------------------------------------------------------------------------------

/// What the native policy does with several desktops on one output. Deliberately only one
/// variant: this is the open B6 question (desktop identity across monitor unplug/replug).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeMultipleDesktops {
    UndecidedB6,
}

/// Several pinned workspaces on one output, e.g. after DP-1 is unplugged and its pinned desktop
/// is moved onto eDP-1, or after logging in without DP-1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipleDesktops {
    pub first: usize,
    pub others: Vec<usize>,
    /// Today: the helper moves the others' visible normal windows to `first`, then unpins the
    /// others, and the compositor persists that unpin (`pinned_workspaces` is rewritten).
    pub current_helper: CurrentHelperMerge,
    pub native: NativeMultipleDesktops,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentHelperMerge {
    pub move_to: usize,
    /// (workspace, windows the helper moves from it): visible and not big.
    pub moved_windows: Vec<(usize, Vec<WindowId>)>,
    pub unpin: Vec<usize>,
    /// The unpin is persisted by the compositor: a side effect that outlives the session.
    pub persists_unpin: bool,
}

/// Describes the situation and today's response, without choosing the native one.
pub fn multiple_desktops(snap: &OutputSnapshot) -> Option<MultipleDesktops> {
    let DesktopSelection::Conflict { first, others } = select_desktop(snap) else {
        return None;
    };
    let moved_windows = others
        .iter()
        .map(|&i| {
            let ids = snap.workspaces[i]
                .visible()
                .filter(|w| !w.state.is_big())
                .map(|w| w.id)
                .collect();
            (i, ids)
        })
        .collect();
    Some(MultipleDesktops {
        first,
        current_helper: CurrentHelperMerge {
            move_to: first,
            moved_windows,
            unpin: others.clone(),
            persists_unpin: true,
        },
        others,
        native: NativeMultipleDesktops::UndecidedB6,
    })
}

// ---------------------------------------------------------------------------------------------
// desired invariants (checked, not assumed)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    NoDesktop,
    SeveralDesktops(Vec<usize>),
    /// A visible big window shares its workspace with other visible windows.
    BigWindowShares { workspace: usize, window: WindowId },
    /// A visible big window sits on the desktop.
    BigWindowOnDesktop { workspace: usize, window: WindowId },
}

/// The Spaces outcomes the policy aims for, reported against a snapshot. An empty result means
/// the output looks like Spaces should; anything else is a state the real compositor *can* be
/// in and the policy would want to correct.
pub fn spaces_violations(snap: &OutputSnapshot) -> Vec<Violation> {
    let mut out = Vec::new();
    let desktop = match select_desktop(snap) {
        DesktopSelection::Pinned(i) => Some(i),
        DesktopSelection::Unpinned { .. } | DesktopSelection::NoWorkspaces => {
            out.push(Violation::NoDesktop);
            None
        }
        DesktopSelection::Conflict { first, others } => {
            let mut all = vec![first];
            all.extend(others);
            out.push(Violation::SeveralDesktops(all));
            Some(first)
        }
    };
    for (i, ws) in snap.workspaces.iter().enumerate() {
        for w in ws.visible().filter(|w| w.state.is_big()) {
            if desktop == Some(i) {
                out.push(Violation::BigWindowOnDesktop { workspace: i, window: w.id });
            } else if ws.visible_others(w.id) > 0 {
                out.push(Violation::BigWindowShares { workspace: i, window: w.id });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMAL: WindowState = WindowState { maximized: false, fullscreen: false, minimized: false };
    const MAX: WindowState = WindowState { maximized: true, fullscreen: false, minimized: false };
    const FULL: WindowState = WindowState { maximized: false, fullscreen: true, minimized: false };
    const MIN: WindowState = WindowState { maximized: false, fullscreen: false, minimized: true };
    const MAX_MIN: WindowState = WindowState { maximized: true, fullscreen: false, minimized: true };

    fn win(id: WindowId, state: WindowState) -> WindowInfo {
        WindowInfo { id, state }
    }
    fn ws(windows: Vec<WindowInfo>) -> WorkspaceInfo {
        WorkspaceInfo { windows, ..Default::default() }
    }
    fn desk(windows: Vec<WindowInfo>) -> WorkspaceInfo {
        WorkspaceInfo { pinned: true, windows, ..Default::default() }
    }
    fn out(workspaces: Vec<WorkspaceInfo>, active: usize) -> OutputSnapshot {
        OutputSnapshot { workspaces, active }
    }

    // ----- model ------------------------------------------------------------------------------

    #[test]
    fn model_big_means_maximized_or_fullscreen_and_not_minimized() {
        assert!(MAX.is_big());
        assert!(FULL.is_big());
        assert!(!NORMAL.is_big());
        assert!(!MAX_MIN.is_big());
    }

    #[test]
    fn model_can_hold_states_the_policy_does_not_want() {
        // no desktop, two desktops, a big window sharing: all representable, none assumed away
        let none = out(vec![ws(vec![])], 0);
        assert_eq!(spaces_violations(&none), vec![Violation::NoDesktop]);
        let two = out(vec![desk(vec![]), desk(vec![])], 0);
        assert_eq!(spaces_violations(&two), vec![Violation::SeveralDesktops(vec![0, 1])]);
        let shared = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(
            spaces_violations(&shared),
            vec![Violation::BigWindowShares { workspace: 1, window: 1 }]
        );
    }

    #[test]
    fn model_functions_never_panic_on_bad_indices() {
        let s = out(vec![desk(vec![win(1, MAX)])], 7);
        assert_eq!(on_became_big(&s, 3, 1), None);
        assert_eq!(on_became_big(&s, 0, 99), None);
        assert_eq!(on_stopped_being_big(&s, 3, 1, SmallCause::Restored), None);
        assert_eq!(on_owner_closed(&s, 3, 1, true), None);
        assert_eq!(on_window_moved(&s, 3, 1, MoveOrigin::Unknown), None);
        assert_eq!(classify_workspace(&s, 3), None);
        let empty = OutputSnapshot::default();
        assert_eq!(select_desktop(&empty), DesktopSelection::NoWorkspaces);
        assert!(current_prune_verdicts(&empty).is_empty());
        assert_eq!(current_helper_desktop_to_pin(&empty), None);
    }

    #[test]
    fn intended_well_formed_spaces_output_has_no_violations() {
        let s = out(
            vec![desk(vec![win(1, NORMAL), win(2, MIN)]), ws(vec![win(3, MAX)]), ws(vec![win(4, FULL)])],
            1,
        );
        assert!(spaces_violations(&s).is_empty());
    }

    // ----- 1. desktop selection ---------------------------------------------------------------

    #[test]
    fn current_helper_desktop_is_the_pinned_workspace_wherever_it_sits() {
        let s = out(vec![ws(vec![]), desk(vec![]), ws(vec![])], 0);
        assert_eq!(select_desktop(&s), DesktopSelection::Pinned(1));
    }

    #[test]
    fn current_helper_unpinned_output_falls_back_to_leftmost() {
        let s = out(vec![ws(vec![win(1, NORMAL)]), ws(vec![])], 0);
        assert_eq!(select_desktop(&s), DesktopSelection::Unpinned { fallback: 0 });
    }

    #[test]
    fn current_helper_pins_first_workspace_without_a_big_window() {
        let s = out(vec![ws(vec![win(1, MAX)]), ws(vec![win(2, NORMAL)])], 0);
        assert_eq!(current_helper_desktop_to_pin(&s), Some(1));
        // a minimized big window does not count as shown
        let s = out(vec![ws(vec![win(1, MAX_MIN)])], 0);
        assert_eq!(current_helper_desktop_to_pin(&s), Some(0));
        // nothing to do when one is already pinned
        let s = out(vec![desk(vec![])], 0);
        assert_eq!(current_helper_desktop_to_pin(&s), None);
    }

    // ----- 2. becoming big --------------------------------------------------------------------

    #[test]
    fn current_helper_classifies_big_changes_by_cause() {
        assert_eq!(classify_change(NORMAL, MAX), StateChange::BecameBig(BigCause::Enlarged));
        assert_eq!(classify_change(NORMAL, FULL), StateChange::BecameBig(BigCause::Enlarged));
        assert_eq!(classify_change(MAX_MIN, MAX), StateChange::BecameBig(BigCause::Unminimized));
        assert_eq!(classify_change(MAX, NORMAL), StateChange::StoppedBeingBig(SmallCause::Restored));
        assert_eq!(classify_change(MAX, MAX_MIN), StateChange::StoppedBeingBig(SmallCause::Minimized));
        assert_eq!(classify_change(MAX, FULL), StateChange::Unchanged);
        assert_eq!(classify_change(NORMAL, MIN), StateChange::Unchanged);
    }

    /// Audit case A: maximizing on the desktop gives the window a new Space.
    #[test]
    fn intended_maximizing_on_the_desktop_creates_a_space() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)])], 0);
        assert_eq!(on_became_big(&s, 0, 1), Some(BigDecision::CreateSpace));
        // alone on the desktop still leaves the desktop
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(on_became_big(&s, 0, 1), Some(BigDecision::CreateSpace));
    }

    #[test]
    fn intended_a_window_alone_on_a_non_desktop_workspace_already_has_its_space() {
        let s = out(vec![desk(vec![win(2, NORMAL)]), ws(vec![win(1, FULL), win(3, MIN)])], 1);
        assert_eq!(on_became_big(&s, 1, 1), Some(BigDecision::AlreadyOwnSpace));
    }

    #[test]
    fn intended_maximizing_on_a_shared_non_desktop_workspace_creates_a_space() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(on_became_big(&s, 1, 1), Some(BigDecision::CreateSpace));
    }

    /// Stock COSMIC keeps an empty workspace at the end and the helper reuses it. With
    /// COSMIC_MAC_SPACES=1 there is normally none, but a just-created one (still empty) would be
    /// reused too. Whether native Spaces ever reuse is a B3 decision.
    #[test]
    fn undecided_helper_reuses_a_trailing_empty_workspace() {
        let s = out(vec![desk(vec![win(1, MAX)]), ws(vec![])], 0);
        assert_eq!(on_became_big(&s, 0, 1), Some(BigDecision::ReuseEmptyWorkspace(1)));
        // a trailing workspace holding only minimized windows also counts as empty for reuse
        let s = out(vec![desk(vec![win(1, MAX)]), ws(vec![win(2, MIN)])], 0);
        assert_eq!(on_became_big(&s, 0, 1), Some(BigDecision::ReuseEmptyWorkspace(1)));
    }

    /// Audit case D: a window that maps already maximized lands on the active workspace first,
    /// then gets the same decision as a maximize.
    #[test]
    fn intended_a_window_mapped_maximized_on_the_desktop_gets_a_space() {
        let s = out(vec![desk(vec![win(2, NORMAL), win(1, MAX)])], 0);
        assert_eq!(classify_change(WindowState::default(), MAX), StateChange::BecameBig(BigCause::Enlarged));
        assert_eq!(on_became_big(&s, 0, 1), Some(BigDecision::CreateSpace));
    }

    /// Audit case D, the other half: a *normal* window opened while a Space is showing maps onto
    /// that Space and today nothing moves it (the helper sees no state change and no move).
    #[test]
    fn undecided_a_normal_window_opened_on_a_space_stays_there() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(classify_change(WindowState::default(), NORMAL), StateChange::Unchanged);
        assert_eq!(classify_workspace(&s, 1), Some(WorkspaceKind::SharedWithBig { big: vec![1] }));
    }

    /// Audit case F: two windows become big at once. Each decision is independent here; the
    /// helper's single `pending_new` slot is a mechanics limitation, not policy.
    #[test]
    fn intended_two_simultaneous_maximizes_each_need_their_own_space() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, MAX), win(3, NORMAL)])], 0);
        assert_eq!(on_became_big(&s, 0, 1), Some(BigDecision::CreateSpace));
        assert_eq!(on_became_big(&s, 0, 2), Some(BigDecision::CreateSpace));
    }

    /// Un-minimizing a maximized window looks to the helper exactly like maximizing it.
    #[test]
    fn undecided_unminimizing_a_maximized_window_is_treated_as_maximizing() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)])], 0);
        assert_eq!(classify_change(MAX_MIN, MAX), StateChange::BecameBig(BigCause::Unminimized));
        assert_eq!(on_became_big(&s, 0, 1), Some(BigDecision::CreateSpace));
    }

    // ----- 3. stopping being big --------------------------------------------------------------

    #[test]
    fn intended_restoring_a_space_owner_moves_it_to_the_desktop() {
        let s = out(vec![desk(vec![win(2, NORMAL)]), ws(vec![win(1, NORMAL)])], 1);
        assert_eq!(
            on_stopped_being_big(&s, 1, 1, SmallCause::Restored),
            Some(RestoreDecision::MoveToDesktop { desktop: 0 })
        );
    }

    #[test]
    fn intended_restoring_on_the_desktop_stays() {
        let s = out(vec![desk(vec![win(1, NORMAL)])], 0);
        assert_eq!(
            on_stopped_being_big(&s, 0, 1, SmallCause::Restored),
            Some(RestoreDecision::StayOnDesktop)
        );
    }

    #[test]
    fn current_helper_restore_without_any_workspace_desktop_uses_the_fallback() {
        // unpinned output: the leftmost workspace acts as the desktop
        let s = out(vec![ws(vec![]), ws(vec![win(1, NORMAL)])], 1);
        assert_eq!(
            on_stopped_being_big(&s, 1, 1, SmallCause::Restored),
            Some(RestoreDecision::MoveToDesktop { desktop: 0 })
        );
    }

    /// The helper cannot tell minimizing from restoring: minimizing a Space owner moves it to the
    /// desktop and shows the desktop. Whether minimize should keep the Space is undecided.
    #[test]
    fn undecided_minimizing_a_big_window_currently_moves_it_to_the_desktop() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX_MIN)])], 1);
        assert_eq!(classify_change(MAX, MAX_MIN), StateChange::StoppedBeingBig(SmallCause::Minimized));
        assert_eq!(
            on_stopped_being_big(&s, 1, 1, SmallCause::Minimized),
            Some(RestoreDecision::MoveToDesktop { desktop: 0 })
        );
    }

    // ----- 4. closing an owner ----------------------------------------------------------------

    /// Audit case E: closing the owner of the active Space shows the desktop.
    #[test]
    fn intended_closing_the_owner_of_the_active_space_shows_the_desktop() {
        let s = out(vec![desk(vec![win(2, NORMAL)]), ws(vec![win(1, MAX)])], 1);
        assert_eq!(on_owner_closed(&s, 1, 1, true), Some(CloseDecision::ActivateDesktop(0)));
    }

    #[test]
    fn current_helper_closing_does_nothing_in_the_other_cases() {
        use CloseNothingReason::*;
        // a background Space: stay where the user is
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX)])], 0);
        assert_eq!(on_owner_closed(&s, 1, 1, true), Some(CloseDecision::Nothing(WorkspaceNotActive)));
        // another visible window remains
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(on_owner_closed(&s, 1, 1, true), Some(CloseDecision::Nothing(OtherWindowsRemain)));
        // not big
        let s = out(vec![desk(vec![]), ws(vec![win(1, NORMAL)])], 1);
        assert_eq!(on_owner_closed(&s, 1, 1, false), Some(CloseDecision::Nothing(WasNotBig)));
        // big on the desktop
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(on_owner_closed(&s, 0, 1, true), Some(CloseDecision::Nothing(AlreadyOnDesktop)));
    }

    #[test]
    fn current_helper_minimized_windows_do_not_keep_a_space_showing() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, MIN)])], 1);
        assert_eq!(on_owner_closed(&s, 1, 1, true), Some(CloseDecision::ActivateDesktop(0)));
    }

    // ----- 5. pruning -------------------------------------------------------------------------

    #[test]
    fn current_compositor_prunes_empty_inactive_workspaces() {
        let s = out(vec![desk(vec![]), ws(vec![]), ws(vec![win(1, MAX)]), ws(vec![])], 2);
        assert_eq!(
            current_prune_verdicts(&s),
            vec![
                PruneVerdict::Keep(KeepReason::IndexZero),
                PruneVerdict::Remove,
                PruneVerdict::Keep(KeepReason::Active),
                PruneVerdict::Remove,
            ]
        );
    }

    #[test]
    fn current_compositor_keeps_recent_token_pinned_and_non_empty_workspaces() {
        let recent = WorkspaceInfo { recently_created: true, ..Default::default() };
        let token = WorkspaceInfo { has_activation_token: true, ..Default::default() };
        let s = out(vec![desk(vec![]), recent, token, desk(vec![]), ws(vec![win(1, MIN)])], 0);
        assert_eq!(
            current_prune_verdicts(&s),
            vec![
                PruneVerdict::Keep(KeepReason::IndexZero),
                PruneVerdict::Keep(KeepReason::RecentlyCreated),
                PruneVerdict::Keep(KeepReason::ActivationToken),
                PruneVerdict::Keep(KeepReason::Pinned),
                PruneVerdict::Keep(KeepReason::NotEmpty),
            ]
        );
    }

    /// Audit case E: the emptied Space survives while it is active, and is pruned once the
    /// desktop is shown.
    #[test]
    fn current_compositor_keeps_an_emptied_space_until_it_is_left() {
        let mut s = out(vec![desk(vec![win(2, NORMAL)]), ws(vec![])], 1);
        assert_eq!(current_prune_verdicts(&s)[1], PruneVerdict::Keep(KeepReason::Active));
        s.active = 0;
        assert_eq!(current_prune_verdicts(&s)[1], PruneVerdict::Remove);
    }

    /// `mac_prune` keeps index 0, the helper's desktop is the pinned workspace. When they differ,
    /// an empty index 0 is never pruned. Which one the native policy protects is undecided
    /// (B3/B6).
    #[test]
    fn undecided_prune_protects_index_zero_not_the_desktop() {
        let s = out(vec![ws(vec![]), desk(vec![win(1, NORMAL)])], 1);
        assert_eq!(select_desktop(&s), DesktopSelection::Pinned(1));
        assert_eq!(current_prune_verdicts(&s)[0], PruneVerdict::Keep(KeepReason::IndexZero));
    }

    // ----- 6. identifying a Space -------------------------------------------------------------

    #[test]
    fn current_helper_identifies_spaces_from_their_contents() {
        let s = out(
            vec![
                desk(vec![win(9, MAX)]),
                ws(vec![win(1, MAX), win(5, MIN)]),
                ws(vec![win(2, MAX), win(3, FULL)]),
                ws(vec![win(4, NORMAL)]),
                ws(vec![win(6, MAX_MIN)]),
                ws(vec![]),
            ],
            0,
        );
        assert_eq!(classify_workspace(&s, 0), Some(WorkspaceKind::Desktop));
        assert_eq!(classify_workspace(&s, 1), Some(WorkspaceKind::DedicatedSpace { owner: 1 }));
        assert_eq!(classify_workspace(&s, 2), Some(WorkspaceKind::SharedWithBig { big: vec![2, 3] }));
        assert_eq!(classify_workspace(&s, 3), Some(WorkspaceKind::Ordinary));
        assert_eq!(classify_workspace(&s, 4), Some(WorkspaceKind::Ordinary));
        assert_eq!(classify_workspace(&s, 5), Some(WorkspaceKind::Empty));
    }

    /// Restoring a Space owner in place (before the policy moves it) turns its Space into an
    /// ordinary workspace: today a Space has no identity beyond its contents.
    #[test]
    fn undecided_a_space_has_no_identity_beyond_its_contents() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, NORMAL)])], 1);
        assert_eq!(classify_workspace(&s, 1), Some(WorkspaceKind::Ordinary));
    }

    // ----- 7. moves ---------------------------------------------------------------------------

    /// Audit case C: today a normal window moved into a Space is bounced to the desktop.
    #[test]
    fn current_helper_bounces_a_normal_window_out_of_a_space() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(on_window_moved(&s, 1, 2, MoveOrigin::Unknown), Some(MoveDecision::SendToDesktop(0)));
    }

    #[test]
    fn current_helper_gives_a_big_window_moved_onto_the_desktop_a_space() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)])], 0);
        assert_eq!(
            on_window_moved(&s, 0, 1, MoveOrigin::Unknown),
            Some(MoveDecision::GiveOwnSpace(BigDecision::CreateSpace))
        );
    }

    #[test]
    fn current_helper_leaves_ordinary_moves_alone() {
        // normal window to the desktop, normal window to a workspace without a big window,
        // big window to a workspace of its own
        let s = out(vec![desk(vec![win(1, NORMAL)]), ws(vec![win(2, NORMAL)]), ws(vec![win(3, MAX)])], 0);
        assert_eq!(on_window_moved(&s, 0, 1, MoveOrigin::Unknown), Some(MoveDecision::Respect));
        assert_eq!(on_window_moved(&s, 1, 2, MoveOrigin::Unknown), Some(MoveDecision::Respect));
        assert_eq!(on_window_moved(&s, 2, 3, MoveOrigin::Unknown), Some(MoveDecision::Respect));
    }

    /// Agreed B5 direction: an explicit user move is respected, never bounced back by the Spaces
    /// policy. Recorded here only; nothing applies it yet.
    #[test]
    fn intended_an_explicit_user_move_is_respected() {
        let into_space = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(on_window_moved(&into_space, 1, 2, MoveOrigin::User), Some(MoveDecision::Respect));
        let onto_desktop = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)])], 0);
        assert_eq!(on_window_moved(&onto_desktop, 0, 1, MoveOrigin::User), Some(MoveDecision::Respect));
    }

    #[test]
    fn intended_the_policy_does_not_second_guess_its_own_moves() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(on_window_moved(&s, 1, 2, MoveOrigin::Policy), Some(MoveDecision::Respect));
    }

    /// The respected user move leaves a state the Spaces invariants flag. How (or whether) that
    /// is reconciled later is part of B5.
    #[test]
    fn undecided_a_respected_user_move_can_leave_a_shared_space() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(on_window_moved(&s, 1, 2, MoveOrigin::User), Some(MoveDecision::Respect));
        assert_eq!(
            spaces_violations(&s),
            vec![Violation::BigWindowShares { workspace: 1, window: 1 }]
        );
    }

    // ----- G/H: starting with existing state --------------------------------------------------

    /// Audit cases G/H: the helper never re-evaluates existing windows when it (re)starts, so
    /// existing violations stay until a window changes. The policy can describe them.
    #[test]
    fn current_helper_start_leaves_existing_violations_for_the_policy_to_report() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)]), ws(vec![win(3, MAX), win(4, NORMAL)])], 0);
        assert_eq!(
            spaces_violations(&s),
            vec![
                Violation::BigWindowOnDesktop { workspace: 0, window: 1 },
                Violation::BigWindowShares { workspace: 1, window: 3 },
            ]
        );
    }

    // ----- I: several desktops on one output (B6) ---------------------------------------------

    /// Audit case I: DP-1 unplugged, its pinned desktop moved onto eDP-1. The situation and
    /// today's response are described; the native response is left undecided.
    #[test]
    fn undecided_two_desktops_on_one_output_describes_todays_merge() {
        let s = out(
            vec![
                desk(vec![win(1, NORMAL)]),
                ws(vec![win(2, MAX)]),
                desk(vec![win(3, NORMAL), win(4, MIN), win(5, MAX)]),
            ],
            0,
        );
        let m = multiple_desktops(&s).unwrap();
        assert_eq!(m.first, 0);
        assert_eq!(m.others, vec![2]);
        assert_eq!(m.current_helper.move_to, 0);
        assert_eq!(m.current_helper.moved_windows, vec![(2, vec![3])]);
        assert_eq!(m.current_helper.unpin, vec![2]);
        assert!(m.current_helper.persists_unpin);
        assert_eq!(m.native, NativeMultipleDesktops::UndecidedB6);
    }

    #[test]
    fn current_helper_one_desktop_is_not_a_merge_situation() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX)])], 0);
        assert_eq!(multiple_desktops(&s), None);
    }

    /// Each output is its own snapshot (OutputBound): the same window ids on another output
    /// produce independent decisions.
    #[test]
    fn intended_outputs_are_decided_independently() {
        let edp = out(vec![desk(vec![win(1, MAX)])], 0);
        let dp = out(vec![desk(vec![]), ws(vec![win(1, MAX)])], 1);
        assert_eq!(on_became_big(&edp, 0, 1), Some(BigDecision::CreateSpace));
        assert_eq!(on_became_big(&dp, 1, 1), Some(BigDecision::AlreadyOwnSpace));
    }
}
