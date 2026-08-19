//! The panel's state, and the root view that dispatches to a screen.
//!
//! One view holds everything rather than a tree of entities: the panel
//! is 392×544 and its state is a list, a settings struct and four bits
//! of navigation. Screens are functions over this state, not separate
//! models, which keeps every transition a plain field assignment
//! followed by `cx.notify()`.

use std::time::Instant;

use gpui::*;
use hi5_core::auth::AuthState;
use hi5_core::github::PullRequest;
use hi5_core::poller::PollEvent;
use hi5_core::store::settings::Appearance;
use hi5_core::store::Settings;
use hi5_core::view::Scope;

use crate::actions::*;
use crate::backend::{Backend, CommandResult, Msg};
use crate::decisions;
pub use crate::decisions::Strip;
use crate::platform::tray::Tray;
use crate::theme;
use crate::ui;
use crate::ui::inbox::InboxDelegate;
use crate::ui::repo_filter::RepoFilterDelegate;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::list::ListState;
use gpui_component::ActiveTheme as _;
use gpui_component::IndexPath;
use gpui_component::Root;
use gpui_component::WindowExt as _;

/// Which screen the panel is showing. `Detail` owns its PR outright: the
/// list underneath can be replaced by a poll cycle mid-read, and a
/// detail view that re-derived its PR from an index would jump to a
/// different pull request when that happened.
#[derive(Clone)]
pub enum Screen {
    Inbox,
    Detail(Box<PullRequest>),
    Settings,
}

/// The two pages of Settings: what hi5 does, and which repositories it
/// does it to. Session state, not a setting — it is where you *were*,
/// and it comes back the next time you open Settings in this run.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Repositories,
}

/// What the bottom status bar last had to report. Session-only — it
/// confirms that something *happened*, so it must never survive a
/// relaunch and pretend otherwise.
#[derive(Clone)]
pub enum LastAction {
    Approved {
        repo: String,
        number: u64,
    },
    ApproveFailed {
        repo: String,
        number: u64,
    },
    Skipped {
        repo: String,
        number: u64,
    },
    /// An "Approve all" finished: `approved` of `total` went through.
    ApprovedAll {
        repo: String,
        approved: usize,
        total: usize,
    },
}

/// An "Approve all" in flight: one repository's pull requests, approved
/// one after another through the same command a single Approve uses,
/// so every guard and every result path is the one already tested.
///
/// Sequential rather than fanned out: each result lands as its own
/// `CommandResult` and the next request goes only when the previous
/// has answered, which keeps `busy` meaning what it says and keeps a
/// failure from being buried under successes that follow it.
#[derive(Debug, Clone)]
pub struct Batch {
    pub repo: String,
    /// `(id, number)` still to send, in the order the list shows them.
    pub queue: std::collections::VecDeque<(String, u64)>,
    pub approved: usize,
    pub failed: usize,
    pub total: usize,
}

pub struct Hi5 {
    pub backend: Backend,
    pub tray: Tray,

    pub prs: Vec<PullRequest>,
    pub settings: Settings,
    pub auth: Option<AuthState>,

    pub screen: Screen,
    /// Scope and repo focus live here rather than inside the inbox
    /// screen: a screen is a render function, so anything held there
    /// would reset every time you opened a PR and came back. Both are
    /// mirrored into `settings.session` on every change and restored at
    /// launch — a menu-bar app is relaunched by things that are not the
    /// user deciding they are done focusing. They remain a *view*, not a
    /// rule: the persistent per-repo control is Settings ▸ Repositories,
    /// which mutes a repo out of the queue entirely.
    pub scope: Scope,
    pub focus_repos: Vec<String>,
    /// Bumped every time the panel hides, and mixed into both menus'
    /// element ids.
    ///
    /// A `Popover` keeps its open flag in window element state keyed by
    /// id, and that state survives the window being ordered out — so a
    /// menu left open when the panel hid was still open behind the
    /// screen the user came back to. A new id is a new piece of state,
    /// which is closed.
    pub menu_generation: usize,

    pub last_updated: Option<Instant>,
    pub rate_limited_until: Option<i64>,
    pub poll_error: Option<String>,
    /// A successful poll cycle is stronger proof the credential is valid
    /// than the `/user` health check: GitHub would not run an
    /// authenticated search and hand back private review requests for a
    /// rejected token. This exists purely so the "couldn't verify"
    /// banner cannot contradict real PRs sitting right below it.
    pub verified_by_poll: bool,

    pub last_action: Option<LastAction>,
    pub action_error: Option<String>,
    pub busy: bool,
    /// See [`Batch`]. `Some` from the confirmation's OK until the last
    /// result is in.
    pub batch: Option<Batch>,
    /// A refresh has been asked for and the poller has not answered yet.
    /// The toolbar's refresh button spins meanwhile; every poll event
    /// clears it, because every poll cycle ends in exactly one.
    pub refreshing: bool,
    /// The auth screen's button has been pressed and the backend has not
    /// said what the credential is worth yet. The button spins
    /// meanwhile — `check_auth` shells out to `gh` and calls GitHub, and
    /// a button that did nothing visible for a second read as broken.
    pub checking_auth: bool,
    /// When the last *asked-for* check answered (wall clock, for the
    /// caption on the connect screen). `None` until the user has pressed
    /// Check again once. Without it a check that lands on the same
    /// state — `gh` still not installed — changed nothing on screen, and
    /// the button read as dead.
    pub auth_checked_at: Option<chrono::DateTime<chrono::Local>>,
    /// Which `gh` hi5 is running, per the last check — see
    /// `Msg::GhResolved`. Shown in Settings ▸ Connection and, when there
    /// is none, on the connect screen.
    pub gh_resolution: Option<hi5_core::auth::runner::Resolution>,
    /// The Settings ▸ Connection field for `Settings::gh_path`. An
    /// entity, as every gpui-component input is; its text is committed
    /// to settings on Enter or blur (`commit_gh_path`), not per keystroke.
    pub gh_path_input: Entity<InputState>,
    /// Which page of Settings is showing.
    pub settings_tab: SettingsTab,
    /// The preview binary's one way to photograph the repo filter open:
    /// a `Popover`'s open flag is element state, keyed by an id that
    /// includes `menu_generation`, and `default_open` is the only hook
    /// on its initial value. Never set by the app itself.
    pub preview_filter_open: bool,
    /// Approve is irreversible, so the button stays inert briefly after
    /// the detail view appears — a fast double-click on a list row must
    /// not carry through into a public approval.
    pub armed_at: Option<Instant>,

    pub org_candidates: Vec<String>,
    /// The panel's own focus, so it receives key events. A menu-bar
    /// panel has no other focusable chrome to compete with — the list
    /// below is driven through its state rather than by focusing it, so
    /// arrows work no matter what was clicked last.
    pub focus: FocusHandle,
    /// The queue, as gpui-component's virtualised sectioned `List`.
    /// Sections are repositories; it owns the scroll offset, the row
    /// measurement and the selection.
    pub inbox: Entity<ListState<InboxDelegate>>,
    /// The repo focus filter's own searchable list.
    pub repo_filter: Entity<ListState<RepoFilterDelegate>>,
    /// Held for its side effect: dropping it unsubscribes.
    _blur: Option<Subscription>,
    /// Focus comes home when whatever held it disappears.
    ///
    /// The `List` takes focus on a row mouse-down (gpui focuses the
    /// deepest `track_focus` element under the pointer; the list is
    /// one). Open a pull request that way and the list is no longer
    /// rendered, but the window's focus still names it — and gpui
    /// dispatches every action, from ‹ and from the keyboard alike, to
    /// the node it can find for the focused handle, or to the *window
    /// root* when it cannot. The root sits above this view's `on_action`
    /// handlers, so ‹, Escape, ⌘↵ and ⌘O all went nowhere; the same
    /// keys worked after a keyboard-opened detail, because arrows and ↵
    /// move the selection through the state and never touch focus. gpui
    /// fires `on_focus_lost` at exactly this moment, and it exists for
    /// exactly this: choose where focus should land instead.
    _focus_lost: Subscription,
    /// Which appearance is currently installed.
    ///
    /// Kept so the palette is only *re-installed* when it actually
    /// changes. Setting a gpui global unconditionally on every render
    /// notifies every observer and asks for another render, which is a
    /// loop — and a loop rebuilds transient element state, which is what
    /// left an open `Popover` unable to dismiss.
    installed_dark: Option<bool>,
}

/// How long Approve stays inert after the detail view appears.
pub const ARMING: std::time::Duration = std::time::Duration::from_millis(250);

impl Hi5 {
    pub fn new(backend: Backend, tray: Tray, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = backend.settings();
        let session = settings.session.clone();
        let me = cx.entity().downgrade();
        let inbox = cx
            .new(|cx| ListState::new(InboxDelegate::new(me.clone()), window, cx).searchable(false));
        let repo_filter =
            cx.new(|cx| ListState::new(RepoFilterDelegate::new(me), window, cx).searchable(true));
        let gh_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("found automatically")
                .default_value(settings.gh_path.clone().unwrap_or_default())
                // A path is one line. The Return key carries a "\n" as
                // its character on macOS and in gpui's tests alike, and a
                // single-line input that lets its Enter action propagate
                // (this one does, so the press can be observed) then
                // receives that character as text — and a newline in a
                // single-line input is a panic in layout. Refusing it
                // here is what keeps Enter meaning "commit".
                .validate(|text, _| !text.contains(['\n', '\r']))
        });
        cx.subscribe_in(&gh_path_input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit_gh_path(window, cx);
            }
        })
        .detach();
        let focus = cx.focus_handle();
        let home = focus.clone();
        let focus_lost = cx.on_focus_lost(window, move |_, window, _| window.focus(&home));
        Self {
            focus,
            inbox,
            repo_filter,
            menu_generation: 0,
            _blur: None,
            _focus_lost: focus_lost,
            installed_dark: None,
            prs: backend.cached_inbox(),
            settings,
            auth: None,
            backend,
            tray,
            screen: Screen::Inbox,
            // Restored from the last run. The default when there is
            // nothing to restore is `All`, not `ForYou`: hi5's premise
            // is the shared pile anyone can review, and opening onto a
            // filtered view that is usually empty would recreate the
            // silent-empty-inbox failure.
            scope: if session.for_you {
                Scope::ForYou
            } else {
                Scope::All
            },
            focus_repos: session.focus_repos,
            last_updated: None,
            rate_limited_until: None,
            poll_error: None,
            verified_by_poll: false,
            last_action: None,
            action_error: None,
            busy: false,
            batch: None,
            refreshing: false,
            checking_auth: false,
            auth_checked_at: None,
            gh_resolution: None,
            gh_path_input,
            settings_tab: SettingsTab::default(),
            preview_filter_open: false,
            armed_at: None,
            org_candidates: Vec::new(),
        }
    }

    /// Push the cached queue into the list, so the first frame draws
    /// something rather than an empty inbox.
    ///
    /// Separate from `new` because filling the delegates needs a `&mut
    /// App`, which `new` does not have while it is still building the
    /// value it will be stored as.
    pub fn prime(&mut self, cx: &mut Context<Self>) {
        self.invalidate(cx);
    }

    /// Hide the panel the moment it stops being the active window.
    ///
    /// This is what separates a menu-bar utility from an app: click
    /// anywhere else and it is gone. Registered against the window
    /// rather than the view because it is the *window's* activation that
    /// matters, and it has to run even while a menu is open — a menu
    /// left showing over a hidden panel would reappear with it.
    pub fn hide_on_blur(
        &mut self,
        panel: crate::platform::panel::Panel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._blur = Some(
            cx.observe_window_activation(window, move |this, window, cx| {
                if window.is_window_active() {
                    return;
                }
                let mtm = objc2::MainThreadMarker::new().expect("gpui callback on the main thread");
                panel.hide(mtm);
                // Out with the panel: the highlight means "this is the
                // window you are looking at".
                this.tray.set_active(false);
                // Reopening lands on the inbox with nothing selected and
                // no menu showing, the way a menu-bar panel is expected to
                // behave — it is a glance, not a session you resume.
                this.screen = Screen::Inbox;
                this.clear_selection(window, cx);
                this.close_menus();
                cx.notify();
            }),
        );
    }

    /// Fold one backend message into the state.
    pub fn handle(&mut self, msg: Msg, cx: &mut Context<Self>) {
        // Every cycle ends in one poll event, so any of them is the
        // poller answering.
        if matches!(msg, Msg::Poll(_)) {
            self.refreshing = false;
        }
        match msg {
            // Signed out means signed out: a cycle that was already in
            // flight when the user signed out may still land, and it must
            // not put a queue behind the signed-out screen. (The poller's
            // own cache still takes it, which is what makes signing back
            // in instant.)
            Msg::Poll(PollEvent::InboxUpdated(_)) | Msg::NotifyPrs(_)
                if self.settings.signed_out => {}
            Msg::Poll(PollEvent::InboxUpdated(update)) => {
                self.prs = update.prs;
                self.last_updated = Some(Instant::now());
                // A successful poll proves any rate limit has cleared and
                // supersedes whatever error was showing — except the
                // anomaly this very cycle detected, which travels with
                // the list precisely so it cannot be raced away here.
                self.rate_limited_until = None;
                self.poll_error = update.anomaly;
                self.verified_by_poll = true;
                self.invalidate(cx);
            }
            Msg::Poll(PollEvent::AuthChanged(state)) => {
                if self.checking_auth {
                    self.auth_checked_at = Some(chrono::Local::now());
                }
                self.checking_auth = false;
                self.auth = Some(state);
                // A fresh auth signal is authoritative. Resetting this
                // means a stale "a poll once proved this" from a previous
                // credential can never mask a genuine rejection.
                self.verified_by_poll = false;
                // Connected with nothing to show — signing back in, or
                // a launch before the first cycle — starts from the
                // poller's last assembled inbox rather than an empty
                // list, the way a relaunch does. A full cycle over every
                // watched org can take a minute; that minute should not
                // say "nothing waiting on you". The refresh spinner is
                // already on for it (see `retry_auth`).
                if self.prs.is_empty() && matches!(self.auth, Some(AuthState::Connected { .. })) {
                    self.prs = self.backend.cached_inbox();
                    self.invalidate(cx);
                }
            }
            Msg::Poll(PollEvent::RateLimited(reset_at)) => self.rate_limited_until = Some(reset_at),
            Msg::Poll(PollEvent::PollError(e)) => self.poll_error = Some(e),
            // The poller's own count is superseded by `sync_badge`
            // below, which knows the auth state and the view; the
            // message is only the cue that something changed.
            Msg::Badge(_) => {}
            Msg::GhResolved(r) => self.gh_resolution = Some(r),
            Msg::Notify { title, body } => crate::platform::notify::banner(&title, &body),
            Msg::NotifyPrs(prs) => {
                for pr in prs {
                    crate::platform::notify::banner(
                        &format!("{} needs your review", pr.author),
                        &format!("{} #{} — {}", pr.repo, pr.number, pr.title),
                    );
                }
            }
            Msg::Command(result) => self.handle_command(result, cx),
        }
        self.sync_badge();
        cx.notify();
    }

    fn handle_command(&mut self, result: CommandResult, cx: &mut Context<Self>) {
        match result {
            CommandResult::Approved { id, repo, number } => {
                self.prs.retain(|p| p.id != id);
                self.busy = false;
                self.screen = Screen::Inbox;
                if let Some(batch) = self.batch.as_mut() {
                    batch.approved += 1;
                    self.next_in_batch(cx);
                } else {
                    self.last_action = Some(LastAction::Approved { repo, number });
                }
            }
            CommandResult::ApproveFailed {
                repo,
                number,
                message,
            } => {
                // The PR deliberately stays in the list. Approve is the
                // irreversible action, so a failure that read like a
                // success would be genuinely bad.
                self.busy = false;
                if let Some(batch) = self.batch.as_mut() {
                    // Counted, and the batch goes on: one PR that could
                    // not be approved (merged since, closed since) is not
                    // a reason to leave the rest un-reviewed. The tally
                    // says so at the end.
                    batch.failed += 1;
                    self.next_in_batch(cx);
                } else {
                    self.action_error = Some(format!("Could not approve: {message}"));
                    self.last_action = Some(LastAction::ApproveFailed { repo, number });
                }
            }
            CommandResult::Skipped { id, repo, number } => {
                self.prs.retain(|p| p.id != id);
                self.last_action = Some(LastAction::Skipped { repo, number });
                self.busy = false;
                self.screen = Screen::Inbox;
            }
            CommandResult::SkipFailed { message } => {
                self.action_error = Some(format!("Could not skip: {message}"));
                self.busy = false;
            }
            CommandResult::Orgs(orgs) => self.org_candidates = orgs,
        }
        self.invalidate(cx);
    }

    /// The menu-bar badge counts the list the panel is actually showing
    /// — repo focus applied, then the active segment — not the
    /// account-wide total. Anything else has the menu bar contradicting
    /// the window it opens.
    pub fn sync_badge(&self) {
        let view = hi5_core::view::inbox_view(&self.prs, &self.focus_repos, self.scope);
        self.tray
            .set_badge(decisions::badge(self.auth.as_ref(), view.visible.len()));
    }

    /// Whether Approve may fire right now. See `decisions::may_approve`
    /// for the two guards and why each is load-bearing.
    pub fn may_approve(&self) -> bool {
        decisions::may_approve(
            matches!(self.screen, Screen::Detail(_)),
            self.is_armed(),
            self.busy,
        )
    }

    /// The pull request the list has selected, if any.
    fn selected_pr(&self, cx: &App) -> Option<PullRequest> {
        self.inbox.read(cx).delegate().selected_pr().cloned()
    }

    /// Push the current queue into the list and the repo filter.
    ///
    /// Called when the data, the scope or the repo focus changes — never
    /// per frame. The list keeps its own scroll offset across this, so a
    /// poll landing while the user is reading does not move the page.
    pub fn invalidate(&mut self, cx: &mut Context<Self>) {
        let prs = self.prs.clone();
        let focus = self.focus_repos.clone();
        let scope = self.scope;
        self.inbox.update(cx, |list, cx| {
            list.delegate_mut().set_queue(&prs, &focus, scope);
            cx.notify();
        });
        let counts = hi5_core::view::repo_counts(&self.prs, &self.focus_repos);
        self.repo_filter.update(cx, |list, cx| {
            list.delegate_mut().set_repos(counts, focus);
            cx.notify();
        });
    }

    /// The next selectable row after `from`, walking across section
    /// boundaries. Sections can be empty, so this is a scan rather than
    /// arithmetic on the row index.
    fn step(&self, from: Option<IndexPath>, forward: bool, cx: &App) -> Option<IndexPath> {
        let list = self.inbox.read(cx);
        let delegate = list.delegate();
        let mut all: Vec<IndexPath> = Vec::new();
        for section in 0..gpui_component::list::ListDelegate::sections_count(delegate, cx) {
            for row in 0..gpui_component::list::ListDelegate::items_count(delegate, section, cx) {
                all.push(IndexPath::new(row).section(section));
            }
        }
        if all.is_empty() {
            return None;
        }
        let at = from.and_then(|ix| all.iter().position(|c| *c == ix));
        Some(match (at, forward) {
            (None, true) => all[0],
            (None, false) => all[0],
            (Some(i), true) => all[(i + 1).min(all.len() - 1)],
            (Some(i), false) => all[i.saturating_sub(1)],
        })
    }

    fn move_selection(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.step(self.inbox.read(cx).selected_index(), forward, cx);
        let Some(ix) = next else { return };
        self.inbox.update(cx, |list, cx| {
            list.set_selected_index(Some(ix), window, cx);
            list.scroll_to_item(ix, gpui::ScrollStrategy::Center, window, cx);
            cx.notify();
        });
        cx.notify();
    }

    /// Put the view into a known state for a screenshot.
    ///
    /// `dead_code` because the `preview` binary includes this module by
    /// path and is therefore a separate compilation unit — the app
    /// itself never calls this, and never should.
    #[allow(dead_code)]
    ///
    /// Used only by the `preview` binary. It lives here rather than
    /// there because the fields it touches are the view's own, and a
    /// preview that reached around the model to pose it would stop
    /// resembling the app the moment either changed.
    pub fn preview_reset(
        &mut self,
        dark: bool,
        prs: Vec<PullRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        self.screen = Screen::Inbox;
        self.auth = None;
        self.prs = prs;
        self.last_action = None;
        self.action_error = None;
        self.refreshing = false;
        self.checking_auth = false;
        self.auth_checked_at = None;
        // A found gh, so the Settings readout photographs the common
        // case; the "auth-no-gh" pose overrides it.
        self.gh_resolution = Some(hi5_core::auth::runner::Resolution {
            path: "/opt/homebrew/bin/gh".into(),
            runnable: true,
            overridden: false,
        });
        self.last_updated = Some(Instant::now());
        self.preview_filter_open = false;
        self.menu_generation += 1;
        self.focus_repos.clear();
        self.settings.repos.muted.clear();
        self.settings.signed_out = false;
        self.settings.watched_orgs.clear();
        self.org_candidates.clear();
        self.settings_tab = SettingsTab::default();
        self.batch = None;
        self.busy = false;
        window.close_all_dialogs(cx);
        self.clear_selection(window, cx);
        self.preview_scroll(px(0.), cx);
        self.invalidate(cx);
    }

    /// Select the nth row, counting across sections. See
    /// `preview_reset` for why this is allowed to look unused.
    #[allow(dead_code)]
    pub fn preview_select(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut ix = None;
        for _ in 0..=n {
            ix = self.step(ix, true, cx);
        }
        if let Some(ix) = ix {
            self.inbox.update(cx, |list, cx| {
                list.set_selected_index(Some(ix), window, cx);
                cx.notify();
            });
        }
    }

    /// Scroll the queue down by `by`, as a wheel would have.
    /// The Approve-all confirmation for the first drawn section, for the
    /// preview binary.
    pub fn preview_approve_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = hi5_core::view::inbox_view(&self.prs, &self.focus_repos, self.scope);
        let Some(repo) = view.visible.first().map(|p| p.repo.clone()) else {
            return;
        };
        self.approve_all(&ApproveAll { repo }, window, cx);
    }

    pub fn preview_scroll(&mut self, by: Pixels, cx: &mut Context<Self>) {
        self.inbox.update(cx, |list, cx| {
            list.scroll_handle()
                .base_handle()
                .set_offset(point(px(0.), -by));
            cx.notify();
        });
    }

    pub fn save_settings(&mut self) {
        self.backend.save_settings(&self.settings);
    }

    /// Persist how the panel is currently being looked at.
    ///
    /// Kept separate from `save_settings` only so the three call sites
    /// read as one idea: every mutation of scope or repo focus lands in
    /// the file, so a relaunch resumes the same view. See
    /// `store::settings::Session` for why this is worth persisting at
    /// all.
    fn save_session(&mut self) {
        self.settings.session = hi5_core::store::Session {
            for_you: self.scope == Scope::ForYou,
            focus_repos: self.focus_repos.clone(),
        };
        self.save_settings();
    }

    /// Shut both toolbar menus, by giving them fresh element ids.
    ///
    /// See `menu_generation`: a `Popover`'s open flag lives in window
    /// element state keyed by id and outlives the window being ordered
    /// out, so the only reliable way to close one from here is to stop
    /// asking for the same one.
    pub fn close_menus(&mut self) {
        self.menu_generation = self.menu_generation.wrapping_add(1);
    }

    pub fn go(&mut self, screen: Screen, cx: &mut Context<Self>) {
        // Leaving the inbox leaves its menus behind with it.
        self.close_menus();
        if matches!(screen, Screen::Detail(_)) {
            self.armed_at = Some(Instant::now());
            self.action_error = None;
            // Ask for a repaint when the gate expires. Without this the
            // Approve button *is* armed but still draws disabled, because
            // nothing else asks for a frame in those 250ms — the state
            // and the pixels disagree until the user moves the mouse.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ARMING).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        self.screen = screen;
        cx.notify();
    }

    pub fn is_armed(&self) -> bool {
        self.armed_at.is_some_and(|t| t.elapsed() >= ARMING)
    }

    /// ↓ and ↑.
    ///
    /// The list is driven through its state rather than by focusing it:
    /// a menu-bar panel has one focus target, and clicking the toolbar
    /// or dismissing a menu must not silently take the arrows away.
    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Inbox) {
            return;
        }
        self.move_selection(true, window, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Inbox) {
            return;
        }
        self.move_selection(false, window, cx);
    }

    pub fn clear_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inbox.update(cx, |list, cx| {
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
    }

    fn open_selected(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Inbox) {
            return;
        }
        if let Some(pr) = self.selected_pr(cx) {
            self.go(Screen::Detail(Box::new(pr)), cx);
        }
    }

    /// Escape. What it means depends on what is showing: dismiss a menu,
    /// leave a screen, or clear the selection. There is nothing above
    /// the inbox to go back to and the panel hides itself on blur, so at
    /// the top level it only clears.
    /// Escape. What it means depends on what is showing: leave a screen,
    /// or clear the selection. There is nothing above the inbox to go
    /// back to and the panel hides itself on blur, so at the top level
    /// it only clears. A menu takes Escape first — `Popover` handles it
    /// in its own key context.
    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen {
            Screen::Inbox => self.clear_selection(window, cx),
            _ => self.screen = Screen::Inbox,
        }
        cx.notify();
    }

    fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Screen::Inbox, cx);
    }

    fn refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.backend.refresh();
        // The poller ignores a wake during a rate-limit sleep — on
        // purpose, see `poller::run` — so there is nothing to wait for
        // and the strip already says why. Spinning here would promise an
        // answer that is not coming.
        self.refreshing = self.rate_limited_until.is_none();
        cx.notify();
    }

    pub fn set_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.settings_tab = tab;
        cx.notify();
    }

    /// Sign out of hi5. Polling stops (`poller::run` idles on the
    /// setting), the queue and everything derived from it is dropped,
    /// and the panel shows the signed-out screen until "Sign in". The
    /// GitHub CLI's own session is not touched: it is not hi5's, and a
    /// menu-bar app quietly running `gh auth logout` would sign the user
    /// out of their terminal.
    fn sign_out(&mut self, _: &SignOut, _: &mut Window, cx: &mut Context<Self>) {
        self.settings.signed_out = true;
        // Saving wakes the poller, which reads the flag and stops.
        self.save_settings();
        self.auth = Some(AuthState::SignedOut);
        self.prs.clear();
        self.verified_by_poll = false;
        self.poll_error = None;
        self.rate_limited_until = None;
        self.last_action = None;
        self.action_error = None;
        self.refreshing = false;
        self.last_updated = None;
        self.screen = Screen::Inbox;
        self.invalidate(cx);
        self.sync_badge();
        cx.notify();
    }

    /// Whether the inbox is empty because nothing has been fetched yet,
    /// as opposed to empty because there is nothing. True until the
    /// first cycle since launch or sign-in lands, unless something has
    /// already gone wrong (a poll error, a rate limit — the strip says
    /// which). The empty state shows a spinner for this and the words
    /// "nothing waiting on you" for the other; they must never be
    /// confused, because one is a promise about GitHub.
    pub fn loading(&self) -> bool {
        self.prs.is_empty()
            && self.last_updated.is_none()
            && self.poll_error.is_none()
            && self.rate_limited_until.is_none()
    }

    /// The auth screen's one button. Whatever the state says, it means
    /// "try again from the top": sign back in if signed out, mark the
    /// welcome step done, and ask the backend what the credential is
    /// worth now.
    /// The gh path field, committed: what is typed becomes
    /// `Settings::gh_path` (blank is "find it"), is saved, and the
    /// credential is re-checked through it — so the readout beside the
    /// field, and the connect screen, answer at once. A no-op when the
    /// text is what settings already say, so a stray blur is not a save.
    pub fn commit_gh_path(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.gh_path_input.read(cx).value().trim().to_string();
        let new = (!typed.is_empty()).then_some(typed);
        if new == self.settings.gh_path {
            return;
        }
        self.settings.gh_path = new;
        self.save_settings();
        self.checking_auth = true;
        self.backend.check_auth();
        cx.notify();
    }

    pub fn retry_auth(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        if self.settings.signed_out {
            self.settings.signed_out = false;
            // The save below wakes the poller into a cycle; say so.
            self.refreshing = true;
            changed = true;
        }
        self.checking_auth = true;
        // Finishing the welcome step is what stops it reappearing on
        // every launch.
        if !self.settings.has_completed_first_run {
            self.settings.has_completed_first_run = true;
            changed = true;
        }
        if changed {
            // Saving wakes the poller, which — signed in again — cycles.
            self.save_settings();
        }
        self.backend.check_auth();
        cx.notify();
    }

    pub(crate) fn open_settings(
        &mut self,
        _: &OpenSettings,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.backend.discover_orgs();
        self.go(Screen::Settings, cx);
    }

    fn open_external(&mut self, _: &OpenExternal, _: &mut Window, cx: &mut Context<Self>) {
        // No arming gate and no busy check: a read-only navigation, not
        // a mutation.
        if let Screen::Detail(pr) = &self.screen {
            cx.open_url(&pr.url);
        }
    }

    fn approve(&mut self, _: &Approve, _: &mut Window, cx: &mut Context<Self>) {
        if !self.may_approve() {
            return;
        }
        let Screen::Detail(pr) = &self.screen else {
            return;
        };
        let (id, repo, number) = (pr.id.clone(), pr.repo.clone(), pr.number);
        self.busy = true;
        self.backend.approve(id, repo, number);
        cx.notify();
    }

    /// "Approve all" on a section header: ask first, listing exactly what
    /// will be approved, and only then start the batch.
    ///
    /// The list in the dialog is the section as drawn — the repo focus
    /// and the All / For you scope applied — because that is what the
    /// header the button sits on is counting. Nothing is approved on
    /// this click; `start_batch` is the dialog's OK.
    fn approve_all(&mut self, action: &ApproveAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.batch.is_some() {
            return;
        }
        let view = hi5_core::view::inbox_view(&self.prs, &self.focus_repos, self.scope);
        let prs: Vec<PullRequest> = view
            .visible
            .iter()
            .filter(|p| p.repo == action.repo)
            .map(|p| (*p).clone())
            .collect();
        if prs.is_empty() {
            return;
        }
        let repo = action.repo.clone();
        let app = cx.entity().downgrade();
        ui::approve_all::open(repo, prs, app, window, cx);
    }

    /// The confirmation's OK. Approves `prs` in order, one request at a
    /// time; results arrive through `handle_command`.
    pub fn start_batch(&mut self, repo: String, prs: &[PullRequest], cx: &mut Context<Self>) {
        if self.busy || self.batch.is_some() || prs.is_empty() {
            return;
        }
        self.last_action = None;
        self.action_error = None;
        self.batch = Some(Batch {
            repo,
            queue: prs.iter().map(|p| (p.id.clone(), p.number)).collect(),
            approved: 0,
            failed: 0,
            total: prs.len(),
        });
        self.next_in_batch(cx);
    }

    /// Send the next request of the batch, or, when there is none left,
    /// close the batch out with its tally.
    fn next_in_batch(&mut self, cx: &mut Context<Self>) {
        let Some(batch) = self.batch.as_mut() else {
            return;
        };
        if let Some((id, number)) = batch.queue.pop_front() {
            let repo = batch.repo.clone();
            self.busy = true;
            self.backend.approve(id, repo, number);
        } else {
            let done = self.batch.take().expect("checked above");
            self.last_action = Some(LastAction::ApprovedAll {
                repo: done.repo,
                approved: done.approved,
                total: done.total,
            });
            if done.failed > 0 {
                self.action_error = Some(format!(
                    "{} of {} could not be approved; they stay in the list.",
                    done.failed, done.total
                ));
            }
        }
        cx.notify();
    }

    fn skip(&mut self, _: &Skip, _: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Screen::Detail(pr) = &self.screen else {
            return;
        };
        let (id, sha, repo, number) = (
            pr.id.clone(),
            pr.head_sha.clone(),
            pr.repo.clone(),
            pr.number,
        );
        self.busy = true;
        self.backend.skip(id, sha, repo, number);
        cx.notify();
    }

    fn set_scope(&mut self, action: &SetScope, _: &mut Window, cx: &mut Context<Self>) {
        self.scope = if action.for_you {
            Scope::ForYou
        } else {
            Scope::All
        };
        self.invalidate(cx);
        self.sync_badge();
        self.save_session();
        cx.notify();
    }

    pub(crate) fn toggle_repo_focus(
        &mut self,
        action: &ToggleRepoFocus,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_repo_focus_by_name(&action.repo, cx);
    }

    /// Deliberately leaves the filter open: focusing "a repo or a few"
    /// is usually more than one click.
    pub fn toggle_repo_focus_by_name(&mut self, repo: &str, cx: &mut Context<Self>) {
        match self.focus_repos.iter().position(|r| r == repo) {
            Some(i) => {
                self.focus_repos.remove(i);
            }
            None => self.focus_repos.push(repo.to_string()),
        }
        self.invalidate(cx);
        self.sync_badge();
        self.save_session();
        cx.notify();
    }

    pub(crate) fn clear_repo_focus(
        &mut self,
        _: &ClearRepoFocus,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_repos.clear();
        self.invalidate(cx);
        self.sync_badge();
        self.save_session();
        cx.notify();
    }

    fn toggle_repo_mute(
        &mut self,
        action: &ToggleRepoMute,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.settings.repos.muted.remove(&action.repo) {
            self.settings.repos.muted.insert(action.repo.clone());
        }
        self.save_settings();
        cx.notify();
    }

    fn toggle_org(&mut self, action: &ToggleOrg, _: &mut Window, cx: &mut Context<Self>) {
        match self
            .settings
            .watched_orgs
            .iter()
            .position(|o| o == &action.org)
        {
            Some(i) => {
                self.settings.watched_orgs.remove(i);
            }
            None => self.settings.watched_orgs.push(action.org.clone()),
        }
        self.save_settings();
        cx.notify();
    }

    fn set_appearance(&mut self, action: &SetAppearance, _: &mut Window, cx: &mut Context<Self>) {
        self.settings.appearance = match action.mode.as_str() {
            "light" => Appearance::Light,
            "dark" => Appearance::Dark,
            _ => Appearance::System,
        };
        self.save_settings();
        cx.notify();
    }

    fn set_poll_interval(
        &mut self,
        action: &SetPollInterval,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.poll_interval_secs = action.secs;
        self.save_settings();
        cx.notify();
    }

    fn toggle_rule(&mut self, action: &ToggleRule, _: &mut Window, cx: &mut Context<Self>) {
        let s = &mut self.settings;
        match action.which.as_str() {
            "hide_already_reviewed" => {
                s.rules.hide_already_reviewed = !s.rules.hide_already_reviewed
            }
            "hide_drafts" => s.rules.hide_drafts = !s.rules.hide_drafts,
            "notifications" => s.notifications_enabled = !s.notifications_enabled,
            "launch_at_login" => {
                s.launch_at_login = !s.launch_at_login;
                crate::platform::autostart::set(s.launch_at_login);
            }
            _ => return,
        }
        self.save_settings();
        cx.notify();
    }
    pub fn strip(&self) -> Option<Strip> {
        decisions::strip(
            self.auth.as_ref(),
            self.rate_limited_until,
            self.poll_error.as_deref(),
            self.verified_by_poll,
            self.last_updated.map(|t| t.elapsed()),
            self.settings.poll_interval_secs,
        )
    }

    pub fn needs_auth(&self) -> bool {
        decisions::needs_auth(self.auth.as_ref(), self.settings.has_completed_first_run)
    }
}

impl Render for Hi5 {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Re-resolved every frame so a live macOS appearance switch is
        // picked up with no listener and no stored state: `System` reads
        // whatever the window reports right now. Only *applied* when it
        // changes — setting it unconditionally notifies every observer
        // and asks for another render, which is a loop, and a loop
        // rebuilds the transient element state an open menu lives in.
        let dark = theme::is_dark(self.settings.appearance, window.appearance());
        if self.installed_dark != Some(dark) {
            self.installed_dark = Some(dark);
            theme::set_mode(dark, window, cx);
        }

        // Settings is reachable from the connect screen — it is where
        // "where is gh" gets answered — so it wins over needs_auth here.
        let body = if self.needs_auth() && !matches!(self.screen, Screen::Settings) {
            ui::auth::render(self, cx).into_any_element()
        } else {
            match self.screen.clone() {
                Screen::Inbox => ui::inbox::render(self, window, cx).into_any_element(),
                Screen::Detail(pr) => ui::detail::render(self, &pr, window, cx).into_any_element(),
                Screen::Settings => ui::settings::render(self, window, cx).into_any_element(),
            }
        };

        div()
            .size_full()
            .track_focus(&self.focus)
            // Every command arrives as an action, whether it came from a
            // keystroke (see `actions::bind`) or a menu item — a
            // `PopupMenu`'s items *are* actions. One handler each,
            // instead of a match over raw keystroke strings.
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::sign_out))
            .on_action(cx.listener(Self::open_external))
            .on_action(cx.listener(Self::approve))
            .on_action(cx.listener(Self::approve_all))
            .on_action(cx.listener(Self::skip))
            .on_action(cx.listener(Self::set_scope))
            .on_action(cx.listener(Self::toggle_repo_focus))
            .on_action(cx.listener(Self::clear_repo_focus))
            .on_action(cx.listener(Self::toggle_repo_mute))
            .on_action(cx.listener(Self::toggle_org))
            .on_action(cx.listener(Self::set_appearance))
            .on_action(cx.listener(Self::set_poll_interval))
            .on_action(cx.listener(Self::toggle_rule))
            .on_action(|_: &crate::actions::Quit, _, cx| cx.quit())
            .overflow_hidden()
            .rounded(theme::WINDOW_RADIUS)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            // One line height for everything, stock controls included.
            // See `ui::LINE` for the pixel this buys inside buttons.
            .line_height(ui::LINE)
            .child(body)
            // gpui-component's dialogs (the Approve-all confirmation)
            // are drawn by whoever owns the window's root view — that
            // is this element, not `Root`.
            .children(Root::render_dialog_layer(window, cx))
            // The panel's own outer ring, drawn over everything: an
            // inset border on the container would be covered by the
            // opaque toolbar and footer and survive only down the sides.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(theme::WINDOW_RADIUS)
                    .border_1()
                    .border_color(cx.theme().border),
            )
    }
}
