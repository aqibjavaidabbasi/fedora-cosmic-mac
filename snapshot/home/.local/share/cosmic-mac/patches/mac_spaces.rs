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
//! Stage B3.1 adds the *native* policy (the compositor-owned version, not yet wired): see
//! "native Spaces" below and the `native_` tests.
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
    /// B3: the window that owns this workspace as its dedicated Space, by window identity.
    /// An explicit marker set by the native policy; never derived from an index. It can be
    /// stale (owner closed or moved away); see [`native_markers_to_clear`].
    pub space_owner: Option<WindowId>,
    /// B3: `Workspace::tiling_enabled`. The native policy skips tiling workspaces.
    pub tiling_enabled: bool,
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

// ---------------------------------------------------------------------------------------------
// B3.1: native Spaces (compositor-owned policy, not wired yet)
// ---------------------------------------------------------------------------------------------
//
// The functions above describe what the helper does today. The ones below describe what the
// compositor will do once it owns the policy (B3.3/B3.4), with the decisions agreed for B3:
//
// - maximize gets its own Space; fullscreen is not part of B3 (kind disabled);
// - everything is off unless `COSMIC_MAC_NATIVE_SPACES` is on ([`NativeSpacesConfig`]);
// - only *intents* (a client, the user or a tool asking to maximize/unmaximize) produce
//   decisions; the compositor's own internal maximize/unmaximize changes (stacking, tiling,
//   drag-to-unmaximize, moves) never do ([`ChangeOrigin`]);
// - a Space is identified by an explicit owner marker holding a *window identity*
//   ([`WorkspaceInfo::space_owner`]), never by a workspace index;
// - a new Space is always appended; an empty workspace is never reused;
// - tiling workspaces are skipped;
// - while a gesture is tracking, a decision may move a window but never activates a workspace
//   ([`activation`]);
// - restore returns a Space owner to the pinned desktop ([`restore_target`]);
// - a minimized Space owner stays in its Space.
//
// Indices in the results refer to the snapshot passed in; the caller resolves them to
// workspace handles immediately, in the same call, before anything can change the list.

/// What the native policy is allowed to do. Built once from the session switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeSpacesConfig {
    /// `COSMIC_MAC_NATIVE_SPACES=1`. Off by default: then nothing below produces an action.
    pub enabled: bool,
    /// Maximize gets its own Space.
    pub maximize: bool,
    /// Fullscreen gets its own Space. Always false in B3.
    pub fullscreen: bool,
}

impl NativeSpacesConfig {
    /// The B3 configuration for a given value of the `COSMIC_MAC_NATIVE_SPACES` switch:
    /// maximize only, fullscreen never.
    pub fn b3(native_spaces_switch: bool) -> Self {
        NativeSpacesConfig {
            enabled: native_spaces_switch,
            maximize: true,
            fullscreen: false,
        }
    }
}

impl Default for NativeSpacesConfig {
    /// Off.
    fn default() -> Self {
        Self::b3(false)
    }
}

/// Which kind of "big" a request is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigKind {
    Maximize,
    Fullscreen,
}

/// Where a state change came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOrigin {
    /// Someone asked for it: the client (xdg/X11 request), the user (button, double-click,
    /// keyboard, window menu, drag to the top edge) or a tool (toplevel-management).
    Intent,
    /// The compositor did it as a side effect: stacking toggles, tiling, moving a window into a
    /// tiling workspace, drag-to-unmaximize, re-maximizing after a move.
    Internal,
}

/// Whether a decision may switch workspaces now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    Now,
    /// A gesture is tracking on this output: the user's swipe keeps control.
    No,
}

/// Activation primitive: never take a swipe away from the user.
pub fn activation(gesture_tracking: bool) -> Activation {
    if gesture_tracking { Activation::No } else { Activation::Now }
}

/// Where a restored Space owner goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreTarget {
    /// The output's pinned desktop. With several pinned workspaces (the open B6 case) the first
    /// one, as the helper does today.
    Desktop(usize),
    /// No pinned workspace on this output: nothing is moved.
    NoDesktop,
}

/// Restore-target primitive: the pinned desktop only (no leftmost fallback).
pub fn restore_target(snap: &OutputSnapshot) -> RestoreTarget {
    match select_desktop(snap) {
        DesktopSelection::Pinned(i) => RestoreTarget::Desktop(i),
        DesktopSelection::Conflict { first, .. } => RestoreTarget::Desktop(first),
        DesktopSelection::Unpinned { .. } | DesktopSelection::NoWorkspaces => {
            RestoreTarget::NoDesktop
        }
    }
}

/// The state of a workspace's owner marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerStatus {
    /// No marker.
    Unmarked,
    /// The owner is on this workspace (minimized counts): a valid Space.
    Valid { owner: WindowId },
    /// The owner is not on this workspace any more (closed, or moved away, possibly to another
    /// output). The marker must be cleared.
    OwnerAway { owner: WindowId },
    /// The same owner is also marked on an earlier workspace of this output. Only the first
    /// valid marker counts; this one must be cleared.
    DuplicateOwner { owner: WindowId },
    /// The workspace is a desktop (pinned). A desktop is never a Space; clear the marker.
    OnDesktop { owner: WindowId },
}

impl MarkerStatus {
    pub fn must_clear(&self) -> bool {
        !matches!(self, MarkerStatus::Unmarked | MarkerStatus::Valid { .. })
    }
}

/// Marker validation for every workspace of an output, in order.
pub fn native_marker_status(snap: &OutputSnapshot) -> Vec<MarkerStatus> {
    let mut seen: Vec<WindowId> = Vec::new();
    snap.workspaces
        .iter()
        .map(|ws| {
            let Some(owner) = ws.space_owner else {
                return MarkerStatus::Unmarked;
            };
            if ws.pinned {
                MarkerStatus::OnDesktop { owner }
            } else if !ws.contains(owner) {
                MarkerStatus::OwnerAway { owner }
            } else if seen.contains(&owner) {
                MarkerStatus::DuplicateOwner { owner }
            } else {
                seen.push(owner);
                MarkerStatus::Valid { owner }
            }
        })
        .collect()
}

/// The workspaces whose marker the compositor must clear (indices into this snapshot).
pub fn native_markers_to_clear(snap: &OutputSnapshot) -> Vec<usize> {
    native_marker_status(snap)
        .iter()
        .enumerate()
        .filter(|(_, s)| s.must_clear())
        .map(|(i, _)| i)
        .collect()
}

/// The workspace whose valid marker names `window`, if any. Ownership is looked up by window
/// identity; the index is only where that workspace happens to be in this snapshot.
pub fn native_space_of(snap: &OutputSnapshot, window: WindowId) -> Option<usize> {
    native_marker_status(snap)
        .iter()
        .position(|s| *s == MarkerStatus::Valid { owner: window })
}

/// Why a native decision does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoAction {
    /// `COSMIC_MAC_NATIVE_SPACES` is off.
    Disabled,
    /// This kind (fullscreen in B3) does not get Spaces.
    KindDisabled,
    /// An internal change, not a request.
    Internal,
    /// The window's workspace has tiling enabled.
    TilingWorkspace,
    /// The window already owns a valid Space.
    AlreadyOwnsSpace,
    /// The window does not own a Space, so there is nothing to undo.
    NotASpaceOwner,
    /// No pinned desktop to return to.
    NoDesktop,
    /// The owner is minimized: it stays in its Space.
    MinimizedOwnerStays,
    /// The Space is not the active workspace: the user is elsewhere; stay there.
    SpaceNotActive,
    /// Other visible windows remain in the Space.
    OtherWindowsRemain,
    /// The snapshot does not have the window on the given workspace.
    InvalidInput,
}

/// A native policy decision. Workspace indices refer to the snapshot passed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeDecision {
    Nothing(NoAction),
    /// Append a new workspace to this output, move the window there, maximize it, mark the new
    /// workspace as its Space, and activate it if `activate == Now`.
    CreateSpace { activate: Activation },
    /// (Only with `allow_reuse`, which native Spaces never use.) Move the window to this empty
    /// workspace, mark it, and activate it if `activate == Now`.
    ReuseEmptyWorkspace { workspace: usize, activate: Activation },
    /// Clear the window's Space marker, move it to the desktop, and activate the desktop if
    /// `activate == Now`. The emptied Space is left to pruning.
    ReturnToDesktop { space: usize, desktop: usize, activate: Activation },
    /// Clear the marker of this Space (its owner closed), and activate the desktop if
    /// `activate` is `Some(Now)`.
    OwnerClosed { space: usize, desktop: Option<usize>, activate: Option<Activation> },
}

/// Common gate: switch, kind, origin. `None` means "go on".
fn native_gate(config: &NativeSpacesConfig, kind: BigKind, origin: ChangeOrigin) -> Option<NoAction> {
    if !config.enabled {
        return Some(NoAction::Disabled);
    }
    let kind_on = match kind {
        BigKind::Maximize => config.maximize,
        BigKind::Fullscreen => config.fullscreen,
    };
    if !kind_on {
        return Some(NoAction::KindDisabled);
    }
    if origin == ChangeOrigin::Internal {
        return Some(NoAction::Internal);
    }
    None
}

/// A window on workspace `ws` is asked to become big (B3.3).
///
/// - Already the valid owner of a Space → nothing.
/// - Otherwise → a new Space is appended (also when the window is already alone on an
///   unmarked workspace: Spaces are only ever created, never adopted or reused, unless
///   `allow_reuse` is set, which native Spaces do not do).
pub fn native_on_big_request(
    snap: &OutputSnapshot,
    config: &NativeSpacesConfig,
    ws: usize,
    window: WindowId,
    kind: BigKind,
    origin: ChangeOrigin,
    gesture_tracking: bool,
    allow_reuse: bool,
) -> NativeDecision {
    if let Some(no) = native_gate(config, kind, origin) {
        return NativeDecision::Nothing(no);
    }
    let Some(here) = snap.workspace(ws).filter(|w| w.contains(window)) else {
        return NativeDecision::Nothing(NoAction::InvalidInput);
    };
    if here.tiling_enabled {
        return NativeDecision::Nothing(NoAction::TilingWorkspace);
    }
    if native_space_of(snap, window) == Some(ws) {
        return NativeDecision::Nothing(NoAction::AlreadyOwnsSpace);
    }
    let activate = activation(gesture_tracking);
    if allow_reuse {
        let last = snap.workspaces.len() - 1;
        let w = &snap.workspaces[last];
        if last != ws && !w.pinned && w.is_empty() && !w.tiling_enabled && w.space_owner.is_none() {
            return NativeDecision::ReuseEmptyWorkspace { workspace: last, activate };
        }
    }
    NativeDecision::CreateSpace { activate }
}

/// A window on workspace `ws` is asked to stop being big (B3.4). Only a valid Space owner is
/// moved: a window that was maximized without a Space (switch off at the time, tiling, ...) is
/// left where it is.
pub fn native_on_restore_request(
    snap: &OutputSnapshot,
    config: &NativeSpacesConfig,
    ws: usize,
    window: WindowId,
    kind: BigKind,
    origin: ChangeOrigin,
    gesture_tracking: bool,
) -> NativeDecision {
    if let Some(no) = native_gate(config, kind, origin) {
        return NativeDecision::Nothing(no);
    }
    if snap.workspace(ws).filter(|w| w.contains(window)).is_none() {
        return NativeDecision::Nothing(NoAction::InvalidInput);
    }
    if native_space_of(snap, window) != Some(ws) {
        return NativeDecision::Nothing(NoAction::NotASpaceOwner);
    }
    match restore_target(snap) {
        RestoreTarget::NoDesktop => NativeDecision::Nothing(NoAction::NoDesktop),
        RestoreTarget::Desktop(desktop) => NativeDecision::ReturnToDesktop {
            space: ws,
            desktop,
            activate: activation(gesture_tracking),
        },
    }
}

/// A Space owner was minimized or unminimized: it stays in its Space. (Minimize is never a
/// restore.)
pub fn native_on_minimize_change(snap: &OutputSnapshot, ws: usize, window: WindowId) -> NativeDecision {
    if snap.workspace(ws).filter(|w| w.contains(window)).is_none() {
        return NativeDecision::Nothing(NoAction::InvalidInput);
    }
    if native_space_of(snap, window) == Some(ws) {
        NativeDecision::Nothing(NoAction::MinimizedOwnerStays)
    } else {
        NativeDecision::Nothing(NoAction::NotASpaceOwner)
    }
}

/// The owner of the Space at `ws` closed (B3.4). `snap` is the state *before* the window is
/// removed. The marker is always cleared; the desktop is shown only if the Space was the active
/// workspace and nothing else visible remains there, and never while a gesture is tracking.
pub fn native_on_owner_closed(
    snap: &OutputSnapshot,
    config: &NativeSpacesConfig,
    ws: usize,
    window: WindowId,
    gesture_tracking: bool,
) -> NativeDecision {
    if !config.enabled {
        return NativeDecision::Nothing(NoAction::Disabled);
    }
    let Some(here) = snap.workspace(ws).filter(|w| w.contains(window)) else {
        return NativeDecision::Nothing(NoAction::InvalidInput);
    };
    if native_space_of(snap, window) != Some(ws) {
        return NativeDecision::Nothing(NoAction::NotASpaceOwner);
    }
    let show_desktop = snap.active == ws && here.visible_others(window) == 0;
    let desktop = match restore_target(snap) {
        RestoreTarget::Desktop(d) => Some(d),
        RestoreTarget::NoDesktop => None,
    };
    NativeDecision::OwnerClosed {
        space: ws,
        desktop: desktop.filter(|_| show_desktop),
        activate: (show_desktop && desktop.is_some()).then(|| activation(gesture_tracking)),
    }
}

/// A window was moved by the user (or by anything that is not the Spaces policy). Native
/// Spaces never react: the move is respected (agreed B5 direction). If the window owned the
/// Space it left, that marker is now stale and [`native_markers_to_clear`] reports it.
pub fn native_on_window_moved(origin: MoveOrigin) -> MoveDecision {
    let _ = origin;
    MoveDecision::Respect
}

// ---------------------------------------------------------------------------------------------
// B3.2: infrastructure for the compositor (switch, intent queue, identity map)
// ---------------------------------------------------------------------------------------------

/// `COSMIC_MAC_NATIVE_SPACES` parsing, the same rule as `COSMIC_MAC_SPACES`: on only when the
/// (trimmed) value is exactly `1`. Unset or anything else is off.
pub fn native_spaces_switch(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim() == "1")
}

/// What an intent asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentChange {
    /// Maximize (or, when enabled, fullscreen).
    BecomeBig,
    /// Unmaximize (or, when enabled, unfullscreen).
    Restore,
}

/// A request the native policy will act on. `W` is the compositor's window identity (a weak
/// window handle there; plain ids in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeIntent<W> {
    pub window: W,
    pub kind: BigKind,
    pub change: IntentChange,
    pub origin: ChangeOrigin,
}

/// Intents recorded at request entry points and drained once per event-loop iteration (B3.3).
/// Only real intents for an enabled kind are accepted, so internal maximize/unmaximize changes
/// can never reach the policy, and nothing accumulates while the switch is off.
#[derive(Debug, Clone)]
pub struct IntentQueue<W> {
    items: Vec<NativeIntent<W>>,
}

impl<W> Default for IntentQueue<W> {
    fn default() -> Self {
        IntentQueue { items: Vec::new() }
    }
}

impl<W> IntentQueue<W> {
    /// Record `intent` if the configuration lets the policy act on it. Returns whether it was
    /// recorded.
    pub fn push(&mut self, config: &NativeSpacesConfig, intent: NativeIntent<W>) -> bool {
        if native_gate(config, intent.kind, intent.origin).is_some() {
            return false;
        }
        self.items.push(intent);
        true
    }

    /// Take everything recorded so far, in arrival order.
    pub fn drain(&mut self) -> Vec<NativeIntent<W>> {
        std::mem::take(&mut self.items)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Gives each distinct window (compared by identity, `==`) a [`WindowId`] for one snapshot.
/// Ids are only meaningful within the snapshot they were made for.
#[derive(Debug, Clone)]
pub struct IdentityMap<T> {
    windows: Vec<T>,
    absent: u64,
}

impl<T> Default for IdentityMap<T> {
    fn default() -> Self {
        IdentityMap { windows: Vec::new(), absent: 0 }
    }
}

impl<T: PartialEq + Clone> IdentityMap<T> {
    /// The id of `window`, the same for every equal window.
    pub fn id_of(&mut self, window: &T) -> WindowId {
        match self.windows.iter().position(|w| w == window) {
            Some(i) => i as WindowId,
            None => {
                self.windows.push(window.clone());
                (self.windows.len() - 1) as WindowId
            }
        }
    }

    /// A fresh id that no window has or will get (for an owner that no longer exists).
    pub fn absent(&mut self) -> WindowId {
        self.absent += 1;
        WindowId::MAX - self.absent
    }

    /// The window behind an id, if it is a window's id.
    pub fn window(&self, id: WindowId) -> Option<&T> {
        usize::try_from(id).ok().and_then(|i| self.windows.get(i))
    }
}

// ---------------------------------------------------------------------------------------------
// B3.3: maximize → own Space (classification of maximize sources, the compositor's plan)
// ---------------------------------------------------------------------------------------------

/// Where a maximize came from. The compositor records an intent only at the sources whose
/// [`MaximizeSource::origin`] is `Intent`; the others are listed so the classification is
/// explicit and tested, and are never recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaximizeSource {
    /// xdg-shell `set_maximized` on a mapped window.
    XdgRequest,
    /// X11 `_NET_WM_STATE` maximize request on a mapped window.
    X11Request,
    /// A window that asked to be maximized before it was mapped (xdg or X11), maximized when it
    /// maps.
    MapRequestedMaximized,
    /// zcosmic-toplevel-management `set_maximized` (Dock, app menu, other tools).
    ToplevelManagement,
    /// `Shell::maximize_toggle` maximizing: keyboard shortcut, title-bar double-click and
    /// button, window menu, and dropping a window on the top "maximize" snapping zone.
    UserToggle,
    /// Internal: `toggle_stacking_focused` re-maximizing after rebuilding the stack.
    StackingRemaximize,
    /// Internal: a moved window keeping its maximized state (`move_window`).
    MoveRemaximize,
    /// Internal: the native Spaces policy maximizing a window it placed.
    NativeSpaces,
    /// Internal: leaving fullscreen back to a maximized state.
    FullscreenRestore,
    /// Internal: un-minimizing a window that was maximized.
    Unminimize,
}

impl MaximizeSource {
    pub fn origin(self) -> ChangeOrigin {
        match self {
            MaximizeSource::XdgRequest
            | MaximizeSource::X11Request
            | MaximizeSource::MapRequestedMaximized
            | MaximizeSource::ToplevelManagement
            | MaximizeSource::UserToggle => ChangeOrigin::Intent,
            MaximizeSource::StackingRemaximize
            | MaximizeSource::MoveRemaximize
            | MaximizeSource::NativeSpaces
            | MaximizeSource::FullscreenRestore
            | MaximizeSource::Unminimize => ChangeOrigin::Internal,
        }
    }

    /// The intent to record for this source, if any.
    pub fn intent<W>(self, window: W) -> NativeIntent<W> {
        NativeIntent {
            window,
            kind: BigKind::Maximize,
            change: IntentChange::BecomeBig,
            origin: self.origin(),
        }
    }
}

/// Facts about a drained maximize intent that only the compositor can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MaximizeContext {
    /// A gesture is tracking on the window's output.
    pub gesture_tracking: bool,
    /// A workspace appended to this output would have tiling enabled.
    pub new_space_tiling: bool,
    /// The window can be moved on its own: it is a mapped floating/tiled window, not part of a
    /// stack with other windows, not sticky, not fullscreen and not minimized.
    pub movable: bool,
}

/// Why the compositor will not act, beyond the policy's own reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanSkip {
    Policy(NoAction),
    NotMovable,
    NewSpaceWouldTile,
}

/// What the compositor does with one drained maximize intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaximizePlan {
    /// Leave the window as it is (normal COSMIC maximize already happened).
    Skip(PlanSkip),
    /// Append a Space, move the window there, mark it as owner, keep it maximized, and activate
    /// the Space if `activate == Now`.
    CreateSpace { activate: Activation },
}

/// The compositor's plan for a drained maximize intent (B3.3). Never reuses or adopts a
/// workspace; only acts on intents; skips anything it cannot move safely or would tile.
pub fn native_maximize_plan(
    snap: &OutputSnapshot,
    config: &NativeSpacesConfig,
    ws: usize,
    window: WindowId,
    origin: ChangeOrigin,
    ctx: MaximizeContext,
) -> MaximizePlan {
    match native_on_big_request(
        snap,
        config,
        ws,
        window,
        BigKind::Maximize,
        origin,
        ctx.gesture_tracking,
        false,
    ) {
        NativeDecision::CreateSpace { activate } => {
            if !ctx.movable {
                MaximizePlan::Skip(PlanSkip::NotMovable)
            } else if ctx.new_space_tiling {
                MaximizePlan::Skip(PlanSkip::NewSpaceWouldTile)
            } else {
                MaximizePlan::CreateSpace { activate }
            }
        }
        NativeDecision::Nothing(reason) => MaximizePlan::Skip(PlanSkip::Policy(reason)),
        // not produced with `allow_reuse == false` or for a maximize request
        _ => MaximizePlan::Skip(PlanSkip::Policy(NoAction::InvalidInput)),
    }
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

/// Stage B3.1: the native (compositor-owned) policy. Every test here is an agreed B3 decision.
#[cfg(test)]
mod native_tests {
    use super::*;

    const NORMAL: WindowState = WindowState { maximized: false, fullscreen: false, minimized: false };
    const MAX: WindowState = WindowState { maximized: true, fullscreen: false, minimized: false };
    const MAX_MIN: WindowState = WindowState { maximized: true, fullscreen: false, minimized: true };

    const ON: NativeSpacesConfig = NativeSpacesConfig { enabled: true, maximize: true, fullscreen: false };

    fn win(id: WindowId, state: WindowState) -> WindowInfo {
        WindowInfo { id, state }
    }
    fn ws(windows: Vec<WindowInfo>) -> WorkspaceInfo {
        WorkspaceInfo { windows, ..Default::default() }
    }
    fn desk(windows: Vec<WindowInfo>) -> WorkspaceInfo {
        WorkspaceInfo { pinned: true, windows, ..Default::default() }
    }
    fn space(owner: WindowId, windows: Vec<WindowInfo>) -> WorkspaceInfo {
        WorkspaceInfo { space_owner: Some(owner), windows, ..Default::default() }
    }
    fn tiling(mut w: WorkspaceInfo) -> WorkspaceInfo {
        w.tiling_enabled = true;
        w
    }
    fn out(workspaces: Vec<WorkspaceInfo>, active: usize) -> OutputSnapshot {
        OutputSnapshot { workspaces, active }
    }
    fn maximize(s: &OutputSnapshot, ws: usize, w: WindowId) -> NativeDecision {
        native_on_big_request(s, &ON, ws, w, BigKind::Maximize, ChangeOrigin::Intent, false, false)
    }
    fn restore(s: &OutputSnapshot, ws: usize, w: WindowId) -> NativeDecision {
        native_on_restore_request(s, &ON, ws, w, BigKind::Maximize, ChangeOrigin::Intent, false)
    }

    // ----- configuration ----------------------------------------------------------------------

    #[test]
    fn native_config_is_off_by_default_and_never_includes_fullscreen() {
        assert_eq!(NativeSpacesConfig::default(), NativeSpacesConfig::b3(false));
        assert!(!NativeSpacesConfig::default().enabled);
        assert_eq!(NativeSpacesConfig::b3(true), ON);
        assert!(!NativeSpacesConfig::b3(true).fullscreen);
    }

    #[test]
    fn native_switch_off_does_nothing() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        let off = NativeSpacesConfig::default();
        let d = native_on_big_request(&s, &off, 0, 1, BigKind::Maximize, ChangeOrigin::Intent, false, false);
        assert_eq!(d, NativeDecision::Nothing(NoAction::Disabled));
        let s = out(vec![desk(vec![]), space(1, vec![win(1, NORMAL)])], 1);
        let d = native_on_restore_request(&s, &off, 1, 1, BigKind::Maximize, ChangeOrigin::Intent, false);
        assert_eq!(d, NativeDecision::Nothing(NoAction::Disabled));
        assert_eq!(native_on_owner_closed(&s, &off, 1, 1, false), NativeDecision::Nothing(NoAction::Disabled));
    }

    #[test]
    fn native_maximize_disabled_does_nothing() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        let no_max = NativeSpacesConfig { maximize: false, ..ON };
        let d = native_on_big_request(&s, &no_max, 0, 1, BigKind::Maximize, ChangeOrigin::Intent, false, false);
        assert_eq!(d, NativeDecision::Nothing(NoAction::KindDisabled));
    }

    #[test]
    fn native_fullscreen_is_disabled_in_b3() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        let b3 = NativeSpacesConfig::b3(true);
        let d = native_on_big_request(&s, &b3, 0, 1, BigKind::Fullscreen, ChangeOrigin::Intent, false, false);
        assert_eq!(d, NativeDecision::Nothing(NoAction::KindDisabled));
        let s = out(vec![desk(vec![]), space(1, vec![win(1, NORMAL)])], 1);
        let d = native_on_restore_request(&s, &b3, 1, 1, BigKind::Fullscreen, ChangeOrigin::Intent, false);
        assert_eq!(d, NativeDecision::Nothing(NoAction::KindDisabled));
    }

    // ----- intent vs internal -----------------------------------------------------------------

    #[test]
    fn native_internal_changes_never_decide_anything() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)])], 0);
        let d = native_on_big_request(&s, &ON, 0, 1, BigKind::Maximize, ChangeOrigin::Internal, false, false);
        assert_eq!(d, NativeDecision::Nothing(NoAction::Internal));
        // drag-to-unmaximize (or a stacking toggle) on a Space owner is internal: no restore
        let s = out(vec![desk(vec![]), space(1, vec![win(1, NORMAL)])], 1);
        let d = native_on_restore_request(&s, &ON, 1, 1, BigKind::Maximize, ChangeOrigin::Internal, false);
        assert_eq!(d, NativeDecision::Nothing(NoAction::Internal));
    }

    #[test]
    fn native_intents_do_decide() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(maximize(&s, 0, 1), NativeDecision::CreateSpace { activate: Activation::Now });
        let s = out(vec![desk(vec![]), space(1, vec![win(1, NORMAL)])], 1);
        assert_eq!(
            restore(&s, 1, 1),
            NativeDecision::ReturnToDesktop { space: 1, desktop: 0, activate: Activation::Now }
        );
    }

    // ----- becoming big ------------------------------------------------------------------------

    #[test]
    fn native_maximize_on_the_desktop_or_shared_workspace_creates_a_space() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)]), ws(vec![win(3, MAX), win(4, NORMAL)])], 0);
        assert_eq!(maximize(&s, 0, 1), NativeDecision::CreateSpace { activate: Activation::Now });
        assert_eq!(maximize(&s, 1, 3), NativeDecision::CreateSpace { activate: Activation::Now });
    }

    #[test]
    fn native_already_owned_space_is_left_alone() {
        let s = out(vec![desk(vec![]), space(1, vec![win(1, MAX)])], 1);
        assert_eq!(maximize(&s, 1, 1), NativeDecision::Nothing(NoAction::AlreadyOwnsSpace));
    }

    /// Spaces are only created, never adopted: a window alone on an unmarked workspace still
    /// gets a new Space (the helper would have left it).
    #[test]
    fn native_alone_on_an_unmarked_workspace_still_gets_a_new_space() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX)])], 1);
        assert_eq!(maximize(&s, 1, 1), NativeDecision::CreateSpace { activate: Activation::Now });
        assert_eq!(on_became_big(&s, 1, 1), Some(BigDecision::AlreadyOwnSpace)); // helper differs
    }

    #[test]
    fn native_never_reuses_an_empty_workspace() {
        let s = out(vec![desk(vec![win(1, MAX)]), ws(vec![])], 0);
        assert_eq!(maximize(&s, 0, 1), NativeDecision::CreateSpace { activate: Activation::Now });
        // the reuse primitive exists, but only behind `allow_reuse`, which native Spaces never set
        let d = native_on_big_request(&s, &ON, 0, 1, BigKind::Maximize, ChangeOrigin::Intent, false, true);
        assert_eq!(d, NativeDecision::ReuseEmptyWorkspace { workspace: 1, activate: Activation::Now });
        // and even then, never a desktop, a marked workspace or a tiling one
        let s = out(vec![desk(vec![win(1, MAX)]), space(9, vec![])], 0);
        let d = native_on_big_request(&s, &ON, 0, 1, BigKind::Maximize, ChangeOrigin::Intent, false, true);
        assert_eq!(d, NativeDecision::CreateSpace { activate: Activation::Now });
    }

    #[test]
    fn native_tiling_workspaces_are_skipped() {
        let s = out(vec![tiling(desk(vec![win(1, MAX), win(2, NORMAL)]))], 0);
        assert_eq!(maximize(&s, 0, 1), NativeDecision::Nothing(NoAction::TilingWorkspace));
    }

    // ----- activation -------------------------------------------------------------------------

    #[test]
    fn native_gesture_tracking_never_activates() {
        assert_eq!(activation(false), Activation::Now);
        assert_eq!(activation(true), Activation::No);
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        let d = native_on_big_request(&s, &ON, 0, 1, BigKind::Maximize, ChangeOrigin::Intent, true, false);
        assert_eq!(d, NativeDecision::CreateSpace { activate: Activation::No });
        let s = out(vec![desk(vec![]), space(1, vec![win(1, NORMAL)])], 1);
        let d = native_on_restore_request(&s, &ON, 1, 1, BigKind::Maximize, ChangeOrigin::Intent, true);
        assert_eq!(d, NativeDecision::ReturnToDesktop { space: 1, desktop: 0, activate: Activation::No });
        assert_eq!(
            native_on_owner_closed(&s, &ON, 1, 1, true),
            NativeDecision::OwnerClosed { space: 1, desktop: Some(0), activate: Some(Activation::No) }
        );
    }

    // ----- restore ----------------------------------------------------------------------------

    #[test]
    fn native_restore_returns_the_owner_to_the_pinned_desktop() {
        // the desktop does not have to be leftmost
        let s = out(vec![ws(vec![]), desk(vec![win(2, NORMAL)]), space(1, vec![win(1, NORMAL)])], 2);
        assert_eq!(restore_target(&s), RestoreTarget::Desktop(1));
        assert_eq!(
            restore(&s, 2, 1),
            NativeDecision::ReturnToDesktop { space: 2, desktop: 1, activate: Activation::Now }
        );
    }

    #[test]
    fn native_restore_needs_a_pinned_desktop() {
        let s = out(vec![ws(vec![]), space(1, vec![win(1, NORMAL)])], 1);
        assert_eq!(restore_target(&s), RestoreTarget::NoDesktop);
        assert_eq!(restore(&s, 1, 1), NativeDecision::Nothing(NoAction::NoDesktop));
    }

    #[test]
    fn native_restore_of_a_window_without_a_space_moves_nothing() {
        // maximized on a shared workspace while the switch was off, then restored
        let s = out(vec![desk(vec![]), ws(vec![win(1, NORMAL), win(2, NORMAL)])], 1);
        assert_eq!(restore(&s, 1, 1), NativeDecision::Nothing(NoAction::NotASpaceOwner));
        let s = out(vec![desk(vec![win(1, NORMAL)])], 0);
        assert_eq!(restore(&s, 0, 1), NativeDecision::Nothing(NoAction::NotASpaceOwner));
    }

    #[test]
    fn native_restore_with_two_desktops_uses_the_first_as_today() {
        let s = out(vec![desk(vec![]), desk(vec![]), space(1, vec![win(1, NORMAL)])], 2);
        assert_eq!(restore_target(&s), RestoreTarget::Desktop(0));
    }

    // ----- minimize ---------------------------------------------------------------------------

    #[test]
    fn native_minimized_owner_stays_in_its_space() {
        let s = out(vec![desk(vec![]), space(1, vec![win(1, MAX_MIN)])], 0);
        assert_eq!(native_on_minimize_change(&s, 1, 1), NativeDecision::Nothing(NoAction::MinimizedOwnerStays));
        // the marker stays valid while the owner is minimized in it
        assert_eq!(native_marker_status(&s)[1], MarkerStatus::Valid { owner: 1 });
        assert!(native_markers_to_clear(&s).is_empty());
        // unminimizing it is not a "becoming big" request either: it already owns the Space
        let s = out(vec![desk(vec![]), space(1, vec![win(1, MAX)])], 1);
        assert_eq!(maximize(&s, 1, 1), NativeDecision::Nothing(NoAction::AlreadyOwnsSpace));
        // a minimized window that owns nothing is just not an owner
        let s = out(vec![desk(vec![win(2, MAX_MIN)])], 0);
        assert_eq!(native_on_minimize_change(&s, 0, 2), NativeDecision::Nothing(NoAction::NotASpaceOwner));
    }

    // ----- markers ----------------------------------------------------------------------------

    #[test]
    fn native_marker_validation() {
        let s = out(
            vec![
                desk(vec![win(5, NORMAL)]),
                space(1, vec![win(1, MAX)]),           // valid
                space(2, vec![]),                      // owner closed
                space(3, vec![win(4, NORMAL)]),        // owner moved away
                space(1, vec![win(1, MAX)]),           // duplicate owner (impossible in practice)
                ws(vec![win(6, NORMAL)]),              // unmarked
            ],
            1,
        );
        assert_eq!(
            native_marker_status(&s),
            vec![
                MarkerStatus::Unmarked,
                MarkerStatus::Valid { owner: 1 },
                MarkerStatus::OwnerAway { owner: 2 },
                MarkerStatus::OwnerAway { owner: 3 },
                MarkerStatus::DuplicateOwner { owner: 1 },
                MarkerStatus::Unmarked,
            ]
        );
        assert_eq!(native_markers_to_clear(&s), vec![2, 3, 4]);
    }

    #[test]
    fn native_a_desktop_is_never_a_space() {
        let mut d = desk(vec![win(1, MAX)]);
        d.space_owner = Some(1);
        let s = out(vec![d], 0);
        assert_eq!(native_marker_status(&s), vec![MarkerStatus::OnDesktop { owner: 1 }]);
        assert_eq!(native_markers_to_clear(&s), vec![0]);
        assert_eq!(native_space_of(&s, 1), None);
    }

    #[test]
    fn native_invalid_marker_does_not_count_as_ownership() {
        // window 1 is on a workspace marked for window 2 (stale): window 1 owns nothing
        let s = out(vec![desk(vec![]), space(2, vec![win(1, MAX)])], 1);
        assert_eq!(native_space_of(&s, 1), None);
        assert_eq!(maximize(&s, 1, 1), NativeDecision::CreateSpace { activate: Activation::Now });
        assert_eq!(restore(&s, 1, 1), NativeDecision::Nothing(NoAction::NotASpaceOwner));
    }

    #[test]
    fn native_owner_moved_away_invalidates_its_marker_and_the_move_is_respected() {
        // the user moved owner 1 from its Space to the desktop
        let s = out(vec![desk(vec![win(1, MAX)]), space(1, vec![])], 0);
        assert_eq!(native_on_window_moved(MoveOrigin::User), MoveDecision::Respect);
        assert_eq!(native_marker_status(&s)[1], MarkerStatus::OwnerAway { owner: 1 });
        assert_eq!(native_markers_to_clear(&s), vec![1]);
        assert_eq!(native_space_of(&s, 1), None);
        // and a normal window the user moved into a Space stays there too
        let s = out(vec![desk(vec![]), space(1, vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(native_on_window_moved(MoveOrigin::User), MoveDecision::Respect);
        assert_eq!(native_marker_status(&s)[1], MarkerStatus::Valid { owner: 1 });
    }

    /// Ownership is a window identity: reordering workspaces carries the marker with its
    /// workspace, and a window id that happens to equal an index means nothing.
    #[test]
    fn native_ownership_is_by_identity_not_index() {
        let a = out(vec![desk(vec![]), space(0, vec![win(0, MAX)]), space(2, vec![win(2, MAX)])], 0);
        assert_eq!(native_space_of(&a, 0), Some(1));
        assert_eq!(native_space_of(&a, 2), Some(2));
        // same workspaces, reordered
        let b = out(vec![space(2, vec![win(2, MAX)]), desk(vec![]), space(0, vec![win(0, MAX)])], 1);
        assert_eq!(native_space_of(&b, 0), Some(2));
        assert_eq!(native_space_of(&b, 2), Some(0));
        assert_eq!(maximize(&b, 2, 0), NativeDecision::Nothing(NoAction::AlreadyOwnsSpace));
        assert_eq!(
            restore(&b, 0, 2),
            NativeDecision::ReturnToDesktop { space: 0, desktop: 1, activate: Activation::Now }
        );
    }

    // ----- closing ----------------------------------------------------------------------------

    #[test]
    fn native_owner_closed_on_the_active_space_shows_the_desktop() {
        let s = out(vec![desk(vec![win(2, NORMAL)]), space(1, vec![win(1, MAX)])], 1);
        assert_eq!(
            native_on_owner_closed(&s, &ON, 1, 1, false),
            NativeDecision::OwnerClosed { space: 1, desktop: Some(0), activate: Some(Activation::Now) }
        );
    }

    #[test]
    fn native_owner_closed_elsewhere_or_with_company_only_clears_the_marker() {
        let s = out(vec![desk(vec![]), space(1, vec![win(1, MAX)])], 0);
        assert_eq!(
            native_on_owner_closed(&s, &ON, 1, 1, false),
            NativeDecision::OwnerClosed { space: 1, desktop: None, activate: None }
        );
        let s = out(vec![desk(vec![]), space(1, vec![win(1, MAX), win(2, NORMAL)])], 1);
        assert_eq!(
            native_on_owner_closed(&s, &ON, 1, 1, false),
            NativeDecision::OwnerClosed { space: 1, desktop: None, activate: None }
        );
        // closing a window that owns no Space is not a Space event
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(native_on_owner_closed(&s, &ON, 0, 1, false), NativeDecision::Nothing(NoAction::NotASpaceOwner));
    }

    // ----- outputs ----------------------------------------------------------------------------

    #[test]
    fn native_outputs_are_decided_independently() {
        let edp = out(vec![desk(vec![win(1, MAX)])], 0);
        let dp = out(vec![desk(vec![]), space(7, vec![win(7, MAX)])], 1);
        assert_eq!(maximize(&edp, 0, 1), NativeDecision::CreateSpace { activate: Activation::Now });
        assert_eq!(maximize(&dp, 1, 7), NativeDecision::Nothing(NoAction::AlreadyOwnsSpace));
        // window 7's Space is unknown on eDP-1 (it is not on that output)
        assert_eq!(native_space_of(&edp, 7), None);
        // a Space carried to another output with its workspace keeps a valid marker there
        let edp_after_unplug = out(vec![desk(vec![win(1, NORMAL)]), space(7, vec![win(7, MAX)])], 0);
        assert_eq!(native_space_of(&edp_after_unplug, 7), Some(1));
    }

    #[test]
    fn native_invalid_inputs_never_panic() {
        let s = out(vec![desk(vec![win(1, MAX)])], 5);
        assert_eq!(maximize(&s, 3, 1), NativeDecision::Nothing(NoAction::InvalidInput));
        assert_eq!(maximize(&s, 0, 9), NativeDecision::Nothing(NoAction::InvalidInput));
        assert_eq!(restore(&s, 3, 1), NativeDecision::Nothing(NoAction::InvalidInput));
        assert_eq!(native_on_minimize_change(&s, 3, 1), NativeDecision::Nothing(NoAction::InvalidInput));
        assert_eq!(native_on_owner_closed(&s, &ON, 3, 1, false), NativeDecision::Nothing(NoAction::InvalidInput));
        let empty = OutputSnapshot::default();
        assert!(native_marker_status(&empty).is_empty());
        assert_eq!(restore_target(&empty), RestoreTarget::NoDesktop);
    }

    /// A maximize → restore → maximize sequence produces one Space decision per maximize and
    /// never a second Space for a window that already owns one.
    #[test]
    fn native_repeated_requests_stay_consistent() {
        let start = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(maximize(&start, 0, 1), NativeDecision::CreateSpace { activate: Activation::Now });
        // compositor applied it: Space appended, window moved and marked, Space active
        let in_space = out(vec![desk(vec![]), space(1, vec![win(1, MAX)])], 1);
        assert_eq!(maximize(&in_space, 1, 1), NativeDecision::Nothing(NoAction::AlreadyOwnsSpace));
        assert_eq!(
            restore(&in_space, 1, 1),
            NativeDecision::ReturnToDesktop { space: 1, desktop: 0, activate: Activation::Now }
        );
        // applied: marker cleared, window back on the desktop, empty Space waiting for pruning
        let back = out(vec![desk(vec![win(1, NORMAL)]), ws(vec![])], 0);
        assert_eq!(restore(&back, 0, 1), NativeDecision::Nothing(NoAction::NotASpaceOwner));
        assert_eq!(current_prune_verdicts(&back)[1], PruneVerdict::Remove);
        assert_eq!(maximize(&back, 0, 1), NativeDecision::CreateSpace { activate: Activation::Now });
    }
}

/// Stage B3.2: the switch, the intent queue and the identity map the compositor uses.
#[cfg(test)]
mod infra_tests {
    use super::*;

    const ON: NativeSpacesConfig = NativeSpacesConfig { enabled: true, maximize: true, fullscreen: false };

    fn intent(window: u64, kind: BigKind, change: IntentChange, origin: ChangeOrigin) -> NativeIntent<u64> {
        NativeIntent { window, kind, change, origin }
    }

    #[test]
    fn switch_is_off_unless_exactly_one() {
        assert!(!native_spaces_switch(None));
        assert!(native_spaces_switch(Some("1")));
        assert!(native_spaces_switch(Some(" 1\n")));
        for v in ["", "0", "true", "yes", "on", "2", "11", "1 1"] {
            assert!(!native_spaces_switch(Some(v)), "{v:?} must be off");
        }
        assert_eq!(NativeSpacesConfig::b3(native_spaces_switch(None)), NativeSpacesConfig::default());
    }

    #[test]
    fn queue_accepts_only_intents_for_enabled_kinds() {
        let mut q = IntentQueue::default();
        assert!(q.push(&ON, intent(1, BigKind::Maximize, IntentChange::BecomeBig, ChangeOrigin::Intent)));
        assert!(q.push(&ON, intent(1, BigKind::Maximize, IntentChange::Restore, ChangeOrigin::Intent)));
        // internal changes never enter
        assert!(!q.push(&ON, intent(2, BigKind::Maximize, IntentChange::Restore, ChangeOrigin::Internal)));
        assert!(!q.push(&ON, intent(2, BigKind::Maximize, IntentChange::BecomeBig, ChangeOrigin::Internal)));
        // fullscreen is disabled in B3
        assert!(!q.push(&ON, intent(3, BigKind::Fullscreen, IntentChange::BecomeBig, ChangeOrigin::Intent)));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn queue_records_nothing_while_the_switch_is_off() {
        let mut q = IntentQueue::default();
        let off = NativeSpacesConfig::default();
        assert!(!q.push(&off, intent(1, BigKind::Maximize, IntentChange::BecomeBig, ChangeOrigin::Intent)));
        assert!(q.is_empty());
    }

    #[test]
    fn queue_drains_in_order_and_empties() {
        let mut q = IntentQueue::default();
        q.push(&ON, intent(1, BigKind::Maximize, IntentChange::BecomeBig, ChangeOrigin::Intent));
        q.push(&ON, intent(2, BigKind::Maximize, IntentChange::BecomeBig, ChangeOrigin::Intent));
        q.push(&ON, intent(1, BigKind::Maximize, IntentChange::Restore, ChangeOrigin::Intent));
        let got: Vec<_> = q.drain().into_iter().map(|i| (i.window, i.change)).collect();
        assert_eq!(
            got,
            vec![(1, IntentChange::BecomeBig), (2, IntentChange::BecomeBig), (1, IntentChange::Restore)]
        );
        assert!(q.is_empty());
        assert!(q.drain().is_empty());
    }

    #[test]
    fn identity_map_is_by_identity() {
        let mut m = IdentityMap::default();
        let a = m.id_of(&'a');
        let b = m.id_of(&'b');
        assert_ne!(a, b);
        assert_eq!(m.id_of(&'a'), a);
        assert_eq!(m.window(a), Some(&'a'));
        assert_eq!(m.window(b), Some(&'b'));
        // ids for missing owners never collide with windows or with each other
        let x = m.absent();
        let y = m.absent();
        assert_ne!(x, y);
        assert!(m.window(x).is_none() && m.window(y).is_none());
        assert_ne!(m.id_of(&'c'), x);
    }

    /// The owner-marker lifecycle the compositor applies through these ids: an owner that is a
    /// live window on the workspace is valid; one that closed (absent id) or moved away must be
    /// cleared; a workspace that moves to another output keeps its marker.
    #[test]
    fn owner_marker_lifecycle_through_identity_ids() {
        let mut ids = IdentityMap::default();
        let owner = ids.id_of(&"owner");
        let other = ids.id_of(&"other");
        let ws_with = |w: Vec<WindowId>, mark: Option<WindowId>| WorkspaceInfo {
            windows: w.into_iter().map(|id| WindowInfo { id, state: WindowState::default() }).collect(),
            space_owner: mark,
            ..Default::default()
        };
        let desktop = WorkspaceInfo { pinned: true, ..Default::default() };

        // valid
        let s = OutputSnapshot { workspaces: vec![desktop.clone(), ws_with(vec![owner], Some(owner))], active: 1 };
        assert!(native_markers_to_clear(&s).is_empty());
        // owner moved away (to the desktop)
        let s = OutputSnapshot {
            workspaces: vec![ws_with(vec![owner], None), ws_with(vec![other], Some(owner))],
            active: 0,
        };
        assert_eq!(native_markers_to_clear(&s), vec![1]);
        // owner closed: the compositor can no longer upgrade it and uses an absent id
        let gone = ids.absent();
        let s = OutputSnapshot { workspaces: vec![desktop.clone(), ws_with(vec![], Some(gone))], active: 1 };
        assert_eq!(native_markers_to_clear(&s), vec![1]);
        // the workspace moved to another output: the marker travels with it and stays valid
        let moved = ws_with(vec![owner], Some(owner));
        let s = OutputSnapshot { workspaces: vec![desktop, moved], active: 0 };
        assert!(native_markers_to_clear(&s).is_empty());
        assert_eq!(native_space_of(&s, owner), Some(1));
    }
}

/// Stage B3.3: maximize → own Space. Pure parts of what the compositor does with a maximize.
#[cfg(test)]
mod b33_tests {
    use super::*;

    const NORMAL: WindowState = WindowState { maximized: false, fullscreen: false, minimized: false };
    const MAX: WindowState = WindowState { maximized: true, fullscreen: false, minimized: false };
    const ON: NativeSpacesConfig = NativeSpacesConfig { enabled: true, maximize: true, fullscreen: false };
    const CTX: MaximizeContext = MaximizeContext { gesture_tracking: false, new_space_tiling: false, movable: true };

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
    fn plan(s: &OutputSnapshot, cfg: &NativeSpacesConfig, ws: usize, w: WindowId, ctx: MaximizeContext) -> MaximizePlan {
        native_maximize_plan(s, cfg, ws, w, ChangeOrigin::Intent, ctx)
    }
    const CREATE_NOW: MaximizePlan = MaximizePlan::CreateSpace { activate: Activation::Now };

    // ----- intent wiring ----------------------------------------------------------------------

    #[test]
    fn explicit_maximize_sources_are_intents() {
        for src in [
            MaximizeSource::XdgRequest,
            MaximizeSource::X11Request,
            MaximizeSource::MapRequestedMaximized,
            MaximizeSource::ToplevelManagement,
            MaximizeSource::UserToggle,
        ] {
            assert_eq!(src.origin(), ChangeOrigin::Intent, "{src:?}");
            let i = src.intent(7u64);
            assert_eq!((i.window, i.kind, i.change, i.origin), (7, BigKind::Maximize, IntentChange::BecomeBig, ChangeOrigin::Intent));
        }
    }

    #[test]
    fn internal_maximize_sources_record_nothing() {
        let mut q = IntentQueue::default();
        for src in [
            MaximizeSource::StackingRemaximize,
            MaximizeSource::MoveRemaximize,
            MaximizeSource::NativeSpaces,
            MaximizeSource::FullscreenRestore,
            MaximizeSource::Unminimize,
        ] {
            assert_eq!(src.origin(), ChangeOrigin::Internal, "{src:?}");
            assert!(!q.push(&ON, src.intent(1u64)));
        }
        assert!(q.is_empty());
    }

    #[test]
    fn fullscreen_produces_no_maximize_intent() {
        let mut q = IntentQueue::default();
        let fs = NativeIntent { window: 1u64, kind: BigKind::Fullscreen, change: IntentChange::BecomeBig, origin: ChangeOrigin::Intent };
        assert!(!q.push(&ON, fs));
        assert!(q.is_empty());
    }

    #[test]
    fn switch_off_records_and_plans_nothing() {
        let off = NativeSpacesConfig::default();
        let mut q = IntentQueue::default();
        assert!(!q.push(&off, MaximizeSource::XdgRequest.intent(1u64)));
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(plan(&s, &off, 0, 1, CTX), MaximizePlan::Skip(PlanSkip::Policy(NoAction::Disabled)));
    }

    #[test]
    fn intents_are_processed_fifo_and_once() {
        let mut q = IntentQueue::default();
        for w in [3u64, 1, 2] {
            q.push(&ON, MaximizeSource::XdgRequest.intent(w));
        }
        let order: Vec<u64> = q.drain().into_iter().map(|i| i.window).collect();
        assert_eq!(order, vec![3, 1, 2]);
        assert!(q.drain().is_empty(), "a drained intent is never seen again");
    }

    // ----- the maximize plan ------------------------------------------------------------------

    #[test]
    fn eligible_window_gets_a_new_space() {
        let s = out(vec![desk(vec![win(1, MAX), win(2, NORMAL)])], 0);
        assert_eq!(plan(&s, &ON, 0, 1, CTX), CREATE_NOW);
    }

    #[test]
    fn alone_on_its_workspace_still_gets_a_new_space() {
        let s = out(vec![desk(vec![]), ws(vec![win(1, MAX)])], 1);
        assert_eq!(plan(&s, &ON, 1, 1, CTX), CREATE_NOW);
        // also when an empty workspace is waiting at the end: never reused
        let s = out(vec![desk(vec![win(1, MAX)]), ws(vec![])], 0);
        assert_eq!(plan(&s, &ON, 0, 1, CTX), CREATE_NOW);
    }

    #[test]
    fn already_owned_space_creates_no_duplicate() {
        let owned = WorkspaceInfo { space_owner: Some(1), windows: vec![win(1, MAX)], ..Default::default() };
        let s = out(vec![desk(vec![]), owned], 1);
        assert_eq!(plan(&s, &ON, 1, 1, CTX), MaximizePlan::Skip(PlanSkip::Policy(NoAction::AlreadyOwnsSpace)));
    }

    #[test]
    fn tiling_source_or_tiling_new_space_keeps_normal_maximize() {
        let tiled = WorkspaceInfo { pinned: true, tiling_enabled: true, windows: vec![win(1, MAX)], ..Default::default() };
        let s = out(vec![tiled], 0);
        assert_eq!(plan(&s, &ON, 0, 1, CTX), MaximizePlan::Skip(PlanSkip::Policy(NoAction::TilingWorkspace)));
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        let ctx = MaximizeContext { new_space_tiling: true, ..CTX };
        assert_eq!(plan(&s, &ON, 0, 1, ctx), MaximizePlan::Skip(PlanSkip::NewSpaceWouldTile));
    }

    #[test]
    fn unmovable_window_keeps_normal_maximize() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        let ctx = MaximizeContext { movable: false, ..CTX };
        assert_eq!(plan(&s, &ON, 0, 1, ctx), MaximizePlan::Skip(PlanSkip::NotMovable));
    }

    #[test]
    fn internal_origin_is_never_planned() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(
            native_maximize_plan(&s, &ON, 0, 1, ChangeOrigin::Internal, CTX),
            MaximizePlan::Skip(PlanSkip::Policy(NoAction::Internal))
        );
    }

    // ----- activation -------------------------------------------------------------------------

    #[test]
    fn normal_maximize_activates_the_new_space() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        assert_eq!(plan(&s, &ON, 0, 1, CTX), MaximizePlan::CreateSpace { activate: Activation::Now });
    }

    #[test]
    fn maximize_while_a_gesture_tracks_moves_but_does_not_activate() {
        let s = out(vec![desk(vec![win(1, MAX)])], 0);
        let ctx = MaximizeContext { gesture_tracking: true, ..CTX };
        assert_eq!(plan(&s, &ON, 0, 1, ctx), MaximizePlan::CreateSpace { activate: Activation::No });
    }

    // ----- ownership --------------------------------------------------------------------------

    /// What the compositor's state looks like after applying `CreateSpace`, through the same
    /// identity map it uses: the appended workspace is owned by the same window, by identity.
    #[test]
    fn new_space_is_owned_by_the_same_window_identity() {
        let mut ids = IdentityMap::default();
        let browser = ids.id_of(&"browser");
        let editor = ids.id_of(&"editor");
        let before = out(vec![desk(vec![win(browser, MAX), win(editor, NORMAL)])], 0);
        assert_eq!(plan(&before, &ON, 0, browser, CTX), CREATE_NOW);
        // applied: appended, moved, marked, activated
        let mut space = ws(vec![win(ids.id_of(&"browser"), MAX)]);
        space.space_owner = Some(ids.id_of(&"browser"));
        let after = out(vec![desk(vec![win(editor, NORMAL)]), space], 1);
        assert_eq!(native_space_of(&after, browser), Some(1));
        assert!(native_markers_to_clear(&after).is_empty());
        // the owner is a window identity, not the number 1
        assert_eq!(ids.window(browser), Some(&"browser"));
        // a repeated maximize request for it is now a no-op
        assert_eq!(plan(&after, &ON, 1, browser, CTX), MaximizePlan::Skip(PlanSkip::Policy(NoAction::AlreadyOwnsSpace)));
    }

    #[test]
    fn marker_follows_its_workspace_to_another_output() {
        let mut space = ws(vec![win(1, MAX)]);
        space.space_owner = Some(1);
        let dp = out(vec![desk(vec![]), space.clone()], 1);
        assert_eq!(native_space_of(&dp, 1), Some(1));
        // DP-1 unplugged: the workspace (with its marker) is appended to eDP-1's set
        let edp = out(vec![desk(vec![win(2, NORMAL)]), ws(vec![]), space], 0);
        assert_eq!(native_space_of(&edp, 1), Some(2));
        assert!(native_markers_to_clear(&edp).is_empty());
    }
}
