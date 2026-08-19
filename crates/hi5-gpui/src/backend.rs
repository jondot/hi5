//! The bridge between hi5's domain logic and the UI thread.
//!
//! `hi5_core` is all async, network-bound and `tokio`-flavoured; GPUI
//! renders on the main thread and has its own executor. Rather than try
//! to make one run inside the other, the domain runs on a dedicated
//! `tokio` runtime on its own thread and everything it wants to say
//! arrives on the UI side as a message.
//!
//! That is the whole of the coupling: one channel out of the backend,
//! one runtime handle in. Nothing in `hi5_core` knows a UI exists (see
//! its `PollHost` trait), and nothing in `ui/` knows what a `tokio`
//! runtime is.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use hi5_core::auth::runner::RealRunner;
use hi5_core::auth::AuthState;
use hi5_core::github::{client::Client, GitHubApi, PullRequest};
use hi5_core::poller::{PollEvent, PollHost, PollRuntime};
use hi5_core::store::{self, Settings};

/// Everything the backend can push at the UI, in one type.
///
/// The three non-`Poll` variants exist because `PollHost`'s other
/// methods — notifications and the menu-bar badge — also have to reach
/// the main thread: `tray-icon` wants its title set there, and routing
/// them through the same channel keeps their ordering relative to the
/// inbox update that produced them.
pub enum Msg {
    Poll(PollEvent),
    Notify {
        title: String,
        body: String,
    },
    NotifyPrs(Vec<NotifiablePr>),
    Badge(Option<usize>),
    /// A command finished — `approve`, `skip` or a settings save. The
    /// payload is the PR id when one is involved, so the UI can drop the
    /// row it optimistically removed if the call actually failed.
    Command(CommandResult),
    /// What `gh` resolves to, sent alongside every auth check so the
    /// Settings readout and the connect screen can say which `gh` hi5 is
    /// running — or that it found none.
    GhResolved(hi5_core::auth::runner::Resolution),
}

/// Just enough of a PR to build a banner, owned, because the banner is
/// posted on the UI thread after the borrow the poller had is long gone.
pub struct NotifiablePr {
    pub author: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
}

pub enum CommandResult {
    Approved {
        id: String,
        repo: String,
        number: u64,
    },
    /// No `id`: unlike the success paths, nothing is removed
    /// optimistically for a failed approve — the PR deliberately stays
    /// in the list — so there is nothing to reconcile by id.
    ApproveFailed {
        repo: String,
        number: u64,
        message: String,
    },
    Skipped {
        id: String,
        repo: String,
        number: u64,
    },
    SkipFailed {
        message: String,
    },
    /// Fresh org candidates for the Settings picker.
    Orgs(Vec<String>),
}

/// The `PollHost` the domain sees: a channel with a nice name on it.
struct ChannelHost {
    tx: UnboundedSender<Msg>,
}

impl PollHost for ChannelHost {
    fn notify_prs(&self, prs: &[&PullRequest]) {
        let owned = prs
            .iter()
            .map(|pr| NotifiablePr {
                author: pr.author.login.clone(),
                repo: pr.repo.clone(),
                number: pr.number,
                title: pr.title.clone(),
            })
            .collect();
        let _ = self.tx.unbounded_send(Msg::NotifyPrs(owned));
    }

    fn notify(&self, title: &str, body: &str) {
        let _ = self.tx.unbounded_send(Msg::Notify {
            title: title.to_string(),
            body: body.to_string(),
        });
    }

    fn set_badge(&self, count: Option<usize>) {
        let _ = self.tx.unbounded_send(Msg::Badge(count));
    }

    fn emit(&self, event: PollEvent) {
        let _ = self.tx.unbounded_send(Msg::Poll(event));
    }
}

/// The handle the UI holds.
#[derive(Clone)]
pub struct Backend {
    kind: Kind,
    tx: UnboundedSender<Msg>,
    dir: PathBuf,
}

#[derive(Clone)]
enum Kind {
    /// The real thing: a tokio runtime on its own thread, the poller,
    /// and GitHub at the far end.
    Live {
        rt: tokio::runtime::Handle,
        poll: Arc<PollRuntime<ChannelHost>>,
    },
    /// Records every command and performs none of them.
    ///
    /// This is what headless UI tests build the app on, and it is not a
    /// convenience — `approve` on the live backend posts a public review
    /// on someone else's pull request with whatever credential `gh` has,
    /// and a test that reached it by accident would do exactly that. On
    /// this kind the test asserts the command was *issued*, which is the
    /// UI's whole responsibility.
    Null(Arc<std::sync::Mutex<Vec<Command>>>),
}

/// Everything the UI can ask the backend to do, as data. Recorded by the
/// null backend so a test can assert what a click or a key produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Refresh,
    CheckAuth,
    DiscoverOrgs,
    SaveSettings,
    Approve {
        id: String,
        repo: String,
        number: u64,
    },
    Skip {
        id: String,
        head_sha: String,
        repo: String,
        number: u64,
    },
}

impl Backend {
    /// Start the domain on its own thread and return the handle plus the
    /// stream of everything it will say.
    pub fn start(dir: PathBuf) -> Result<(Self, UnboundedReceiver<Msg>)> {
        let (tx, rx) = unbounded();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("hi5-backend")
            .build()?;
        let handle = runtime.handle().clone();

        let poll = PollRuntime::new(ChannelHost { tx: tx.clone() }, dir.clone());

        // The runtime is moved onto its own thread and parked there for
        // the life of the process. Dropping it would abort every task
        // mid-flight, including a poll cycle holding the state lock.
        let poll_for_loop = poll.clone();
        std::thread::Builder::new()
            .name("hi5-backend".into())
            .spawn(move || {
                runtime.block_on(hi5_core::poller::run(poll_for_loop));
            })?;

        Ok((
            Self {
                kind: Kind::Live { rt: handle, poll },
                tx,
                dir,
            },
            rx,
        ))
    }

    /// A backend that records commands and never touches the network.
    /// See `Kind::Null`. Settings still round-trip through `dir`, which
    /// should be a throwaway directory.
    pub fn null(dir: PathBuf) -> (Self, UnboundedReceiver<Msg>) {
        let (tx, rx) = unbounded();
        (
            Self {
                kind: Kind::Null(Default::default()),
                tx,
                dir,
            },
            rx,
        )
    }

    /// What the null backend has been asked to do, in order. Empty on a
    /// live backend, which does not keep a log.
    pub fn commands(&self) -> Vec<Command> {
        match &self.kind {
            Kind::Null(log) => log.lock().unwrap().clone(),
            Kind::Live { .. } => Vec::new(),
        }
    }

    /// The channel the app listens on. Tests use it to feed the app the
    /// messages a poll cycle would have produced.
    pub fn sender(&self) -> UnboundedSender<Msg> {
        self.tx.clone()
    }

    fn record(&self, command: Command) -> bool {
        match &self.kind {
            Kind::Null(log) => {
                log.lock().unwrap().push(command);
                true
            }
            Kind::Live { .. } => false,
        }
    }

    /// The last assembled inbox, so a panel opened between cycles draws
    /// immediately instead of waiting for one.
    pub fn cached_inbox(&self) -> Vec<PullRequest> {
        match &self.kind {
            Kind::Live { rt, poll } => rt.block_on(async { poll.cache.lock().await.clone() }),
            Kind::Null(_) => Vec::new(),
        }
    }

    pub fn settings(&self) -> Settings {
        store::load_settings(&self.dir).0
    }

    /// The persisted poll state — what the poller has learned about
    /// each repo's branches, among other things — from *this* backend's
    /// directory. The settings screen reads it through here rather than
    /// through `crate::config_dir()`, so a preview or a test built on a
    /// scratch directory sees scratch, not the real install's repos.
    pub fn state(&self) -> hi5_core::store::AppState {
        store::load_state(&self.dir).0
    }

    pub fn save_settings(&self, settings: &Settings) {
        let _ = store::save_settings(&self.dir, settings);
        if self.record(Command::SaveSettings) {
            return;
        }
        // A changed poll interval should take effect now, not after the
        // old one runs out.
        if let Kind::Live { poll, .. } = &self.kind {
            poll.wake.notify_one();
        }
    }

    pub fn refresh(&self) {
        if self.record(Command::Refresh) {
            return;
        }
        if let Kind::Live { poll, .. } = &self.kind {
            poll.wake.notify_one();
        }
    }

    /// Resolve the current credential and report what it is worth. Runs
    /// off the UI thread; the answer arrives as `AuthChanged`.
    pub fn check_auth(&self) {
        if self.record(Command::CheckAuth) {
            return;
        }
        let Kind::Live { rt, .. } = &self.kind else {
            return;
        };
        let tx = self.tx.clone();
        // Signed out is decided by hi5, not by what `gh` would say: the
        // credential is not consulted at all.
        let settings = self.settings();
        let signed_out = settings.signed_out;
        rt.spawn(async move {
            // The user's gh path, if any, before anything runs gh; and
            // what that comes to, for the screens.
            hi5_core::auth::runner::set_gh_override(settings.gh_path.as_deref());
            let _ = tx.unbounded_send(Msg::GhResolved(hi5_core::auth::runner::resolve_gh()));
            let state = if signed_out {
                AuthState::SignedOut
            } else {
                auth_state().await
            };
            let _ = tx.unbounded_send(Msg::Poll(PollEvent::AuthChanged(state)));
        });
    }

    /// Every org the viewer belongs to right now — the candidate list for
    /// the Settings picker, distinct from the *enabled* subset in
    /// `Settings::watched_orgs`.
    pub fn discover_orgs(&self) {
        if self.record(Command::DiscoverOrgs) {
            return;
        }
        let Kind::Live { rt, .. } = &self.kind else {
            return;
        };
        let tx = self.tx.clone();
        rt.spawn(async move {
            let Some((token, _)) = hi5_core::auth::resolve_token(&RealRunner) else {
                return;
            };
            if let Ok(orgs) = Client::new(token).list_orgs().await {
                let _ = tx.unbounded_send(Msg::Command(CommandResult::Orgs(orgs)));
            }
        });
    }

    /// Post a public approving review. The one irreversible thing hi5
    /// does, and the only mutation that reaches GitHub.
    pub fn approve(&self, id: String, repo: String, number: u64) {
        if self.record(Command::Approve {
            id: id.clone(),
            repo: repo.clone(),
            number,
        }) {
            return;
        }
        let Kind::Live { rt, .. } = &self.kind else {
            return;
        };
        let tx = self.tx.clone();
        rt.spawn(async move {
            let msg = match hi5_core::auth::resolve_token(&RealRunner) {
                None => CommandResult::ApproveFailed {
                    repo,
                    number,
                    message: "not signed in to GitHub".into(),
                },
                Some((token, _)) => match Client::new(token).approve(&id).await {
                    Ok(()) => CommandResult::Approved { id, repo, number },
                    Err(e) => CommandResult::ApproveFailed {
                        repo,
                        number,
                        message: e.to_string(),
                    },
                },
            };
            let _ = tx.unbounded_send(Msg::Command(msg));
        });
    }

    /// Mute a PR until its head SHA changes. Local only — nothing is
    /// sent to GitHub.
    pub fn skip(&self, id: String, head_sha: String, repo: String, number: u64) {
        if self.record(Command::Skip {
            id: id.clone(),
            head_sha: head_sha.clone(),
            repo: repo.clone(),
            number,
        }) {
            return;
        }
        let Kind::Live { rt, poll } = &self.kind else {
            return;
        };
        let tx = self.tx.clone();
        let poll = poll.clone();
        let dir = self.dir.clone();
        rt.spawn(async move {
            // Held across the whole read-modify-write: a poll cycle
            // running at this moment does the same round-trip on the same
            // file, and without the lock the mute is read before the
            // poller's mutation and written back after it -- silently
            // lost. See `PollRuntime::state_lock`.
            let _guard = poll.state_lock.lock().await;
            let (mut state, _) = store::load_state(&dir);
            state.muted.insert(id.clone(), head_sha);
            let msg = match store::save_state(&dir, &state) {
                Ok(()) => CommandResult::Skipped { id, repo, number },
                Err(e) => CommandResult::SkipFailed {
                    message: e.to_string(),
                },
            };
            let _ = tx.unbounded_send(Msg::Command(msg));
        });
    }
}

/// Resolve a token and ask GitHub what it is worth.
///
/// Ported line for line from the shipped implementation, including the
/// bug it was fixed for: an earlier version caught *every* `health()`
/// error and reported "token rejected", so a GitHub 503 — or simply
/// being offline — presented as a revoked credential and pushed the user
/// through a needless `gh auth login`. Only a genuine 401 is a
/// rejection. Anything else means connected-but-unverified, and the UI
/// says exactly that rather than pretending the check succeeded.
async fn auth_state() -> AuthState {
    use hi5_core::auth::{gh, health};

    let Some((token, source)) = hi5_core::auth::resolve_token(&RealRunner) else {
        return match gh::detect(&RealRunner) {
            gh::GhState::NotInstalled => AuthState::GhNotInstalled {
                homebrew_available: std::path::Path::new("/opt/homebrew/bin/brew").exists()
                    || std::path::Path::new("/usr/local/bin/brew").exists(),
            },
            gh::GhState::NotAuthenticated => AuthState::GhNotAuthenticated,
            gh::GhState::Ready { .. } => AuthState::NeedsToken,
        };
    };

    match Client::new(token).health().await {
        Ok(h) => {
            let check = health::parse(h.scope_header.as_deref());
            // A classic token without `repo` authenticates fine; GitHub
            // search then silently drops every private result, giving a
            // short inbox and no explanation at all.
            let scopes_adequate = check.is_adequate();
            let scopes = match check {
                health::ScopeCheck::FineGrained => vec![],
                health::ScopeCheck::Classic { scopes, .. } => scopes,
            };
            AuthState::Connected {
                login: h.login,
                source: source.into(),
                scopes,
                scopes_adequate,
                verified: true,
            }
        }
        Err(e) if hi5_core::auth::is_credential_rejected(&e) => AuthState::Disconnected {
            reason: "token expired or revoked".into(),
        },
        // `gh auth status` supplies the login here because it reads gh's
        // local config rather than calling the same `/user` endpoint that
        // just failed.
        Err(_) => AuthState::Connected {
            login: gh::login(&RealRunner).unwrap_or_default(),
            source: source.into(),
            scopes: vec![],
            scopes_adequate: true,
            verified: false,
        },
    }
}
