use crate::error::AppError;
use std::time::Duration;

pub const MAX_BACKOFF_SECS: u64 = 900;

pub struct Backoff {
    base_secs: u64,
    current_secs: u64,
}

impl Backoff {
    pub fn new(base_secs: u64) -> Self {
        Self {
            base_secs,
            current_secs: base_secs,
        }
    }

    /// Updates the healthy-cadence interval without disturbing an
    /// in-flight backoff. A user changing the poll interval is a
    /// preference about the normal cadence, not evidence the remote
    /// recovered -- an elevated `current_secs` means the remote is
    /// genuinely struggling, and only a success (via `next_delay`)
    /// should reset it. The new interval takes effect starting from
    /// the next successful cycle.
    pub fn set_base(&mut self, base_secs: u64) {
        self.base_secs = base_secs;
    }

    /// Doubles on failure up to a 15-minute ceiling, resets on success.
    /// A rate-limit error overrides the schedule entirely and sleeps
    /// until GitHub says the window reopens.
    pub fn next_delay(&mut self, err: Option<&AppError>, now: i64) -> Duration {
        match err {
            None => {
                self.current_secs = self.base_secs;
                Duration::from_secs(self.base_secs)
            }
            Some(AppError::RateLimited(reset_at)) => {
                let wait = (*reset_at - now).max(1) as u64;
                Duration::from_secs(wait)
            }
            Some(_) => {
                self.current_secs = (self.current_secs * 2).min(MAX_BACKOFF_SECS);
                Duration::from_secs(self.current_secs)
            }
        }
    }
}

use crate::auth::{runner::RealRunner, AuthState};
use crate::github::{client::Client, GitHubApi, PullRequest};
use crate::{inbox, notify_diff, query, store};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

/// Everything the poll loop wants to tell the world, as data.
///
/// The Tauri implementation emitted these as four string-keyed webview
/// events. As an enum the shell has to handle each one, and the
/// ordering guarantee that `InboxUpdate` carries its own anomaly (see
/// below) survives being ported rather than being re-derived.
#[derive(Debug, Clone)]
pub enum PollEvent {
    /// A completed cycle. Supersedes any previous rate-limit notice.
    InboxUpdated(InboxUpdate),
    /// The stored credential was rejected. Polling stops until the UI
    /// drives a reconnect.
    AuthChanged(AuthState),
    /// GitHub is rate-limiting; the payload is the reset epoch, not a
    /// preformatted string, so the shell can format it in the user's
    /// locale.
    RateLimited(i64),
    /// A one-off cycle failure, verbatim, for the status strip.
    PollError(String),
}

/// The three things the poll loop cannot do for itself, because each of
/// them is the shell's job: show an OS notification, set the menu-bar
/// badge, and get an event in front of the UI.
///
/// Keeping this to three methods is deliberate. Everything else the old
/// implementation reached into `AppHandle` for -- the config directory,
/// the state lock, the inbox cache, the wake channel -- is not
/// platform-specific at all; it is ordinary `tokio` and `std`, and it
/// lives in [`PollRuntime`] where both shells and the tests can reach it.
pub trait PollHost: Send + Sync + 'static {
    /// One banner per newly-visible PR — for a handful; more than
    /// `notify_diff::BURST` at once arrive as one `notify` instead.
    fn notify_prs(&self, prs: &[&PullRequest]);
    /// A plain banner, used for the corrupt-state-file recovery notice.
    fn notify(&self, title: &str, body: &str);
    /// `None` means disconnected -- the menu bar shows `!` rather than a
    /// count.
    fn set_badge(&self, count: Option<usize>);
    fn emit(&self, event: PollEvent);
}

/// The poll loop's own state: where things are stored, and the three
/// synchronisation primitives shared with whatever else touches them.
///
/// `state_lock` and `cache` are `pub` because the shell's own commands
/// need them: skipping a PR is a load -> mutate -> save on the same file
/// a cycle writes (hence the lock), and opening the panel reads the
/// inbox without waiting for a cycle (hence the cache).
pub struct PollRuntime<H: PollHost> {
    pub host: H,
    pub dir: PathBuf,
    /// Serialises every load -> mutate -> save round-trip on `state.json`.
    ///
    /// Two writers exist: skipping a PR (adding a mute) and a poll cycle
    /// (touching `last_seen`, recording notifications, pruning). Both
    /// read the whole file, mutate in memory and write it back, so
    /// without this a Skip landing mid-cycle is read before the poller's
    /// mutation and overwritten after it -- the mute vanishes and the PR
    /// is still in the inbox on the next refresh.
    pub state_lock: Mutex<()>,
    /// The last assembled inbox, so a panel opened between cycles has
    /// something to draw immediately.
    pub cache: Mutex<Vec<PullRequest>>,
    /// Cuts the poll sleep short -- used by an explicit refresh and by a
    /// settings change (so a new interval takes effect without waiting
    /// out the old one). See `honors_wake` for the case where it must
    /// deliberately *not* take effect.
    pub wake: Notify,
}

impl<H: PollHost> PollRuntime<H> {
    pub fn new(host: H, dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            host,
            dir,
            state_lock: Mutex::new(()),
            cache: Mutex::new(Vec::new()),
            wake: Notify::new(),
        })
    }
}

/// Payload of the `inbox-updated` event.
///
/// The anomaly message rides *inside* this payload rather than being a
/// separate `PollError`. A separate event is unwinnable: `cycle()` would
/// emit it, the loop would then emit `InboxUpdated`, and every UI clears
/// its poll-error on a successful update -- so the warning is set and
/// cleared within a single frame and the user never sees it. Carrying it
/// in the payload makes the two impossible to race, rather than merely
/// ordering them (which the next refactor would silently undo).
#[derive(Debug, Clone)]
pub struct InboxUpdate {
    pub prs: Vec<PullRequest>,
    /// `None` on a healthy cycle -- which is what clears a stale warning
    /// on the frontend.
    pub anomaly: Option<String>,
}

/// Every PR id GitHub returned this cycle, taken from the *raw* per-query
/// results before `inbox::assemble` filters anything out.
///
/// `AppState::prune` drops the `muted`/`notified` bookkeeping for any PR
/// whose `last_seen` is a week stale, on the premise that such a PR is no
/// longer in the results. Touching only the *assembled* list breaks that
/// premise for precisely the PRs the bookkeeping exists for: `assemble`
/// filters muted PRs out, so a muted PR would never be touched, so after
/// seven days its mute is pruned and it walks straight back into the
/// inbox and re-notifies -- even though it appeared in every single
/// response along the way. Spec §7: a mute persists until the head SHA
/// changes, and pruning is for PRs *not seen in results*.
pub fn seen_ids(results: &[Vec<PullRequest>]) -> Vec<&str> {
    results
        .iter()
        .flatten()
        .map(|pr| pr.id.as_str())
        .collect::<Vec<_>>()
}

/// A query that returned nodes but parsed none of them means the API
/// response shape changed, not that the inbox is empty -- the two must
/// never be indistinguishable to the user. `None` covers both the
/// healthy empty case (`raw_count == 0`) and a partial parse
/// (`parsed_count > 0`), which is the tolerated case by design: a
/// partial inbox beats an empty one.
///
/// Kept as a pure function, with no Tauri types in its signature, so
/// the detect-and-emit decision is unit-testable on its own rather than
/// only reachable through `cycle()`'s live `AppHandle`.
pub fn parse_anomaly(raw_count: usize, parsed_count: usize) -> Option<String> {
    if raw_count > 0 && parsed_count == 0 {
        Some(format!(
            "GitHub returned {raw_count} PRs but none could be parsed \
             — the API response shape may have changed"
        ))
    } else {
        None
    }
}

/// Whether a pending sleep may be cut short by a manual refresh or a
/// settings change. A rate-limit sleep must not be: waking early would
/// re-issue the request before GitHub's window reopens, and the user
/// most likely to click refresh is the one already being limited --
/// each click would only make it worse.
///
/// Kept pure, like [`parse_anomaly`], so this decision has unit
/// coverage rather than living only inside `spawn()`'s loop.
pub fn honors_wake(err: Option<&AppError>) -> bool {
    !matches!(err, Some(AppError::RateLimited(_)))
}

/// Whether a poll-cycle error should drop the app to `Disconnected` and
/// pause polling entirely, as opposed to a failure that leaves the
/// credential's status alone (a 5xx, rate limiting, a transport/offline
/// error, or a response-shape anomaly).
///
/// A thin wrapper around `auth::is_credential_rejected` -- not a
/// re-derivation of it -- because `commands::get_auth_state` classifies
/// a `/user` failure with that same predicate, and a poll cycle's
/// failure must mean exactly the same thing that one does. The frontend
/// now treats a *successful* poll as stronger proof of a valid
/// credential than `/user` is (clearing the "couldn't verify" banner);
/// that premise only holds if "this failure means the credential was
/// actually rejected" is decided in one place, not re-derived per call
/// site where it could quietly drift out of sync.
fn is_disconnecting_error(err: &AppError) -> bool {
    crate::auth::is_credential_rejected(err)
}

/// Every org GitHub reports the viewer belongs to, plus their own login
/// (see `query::merge_org_scopes`), used to auto-populate
/// `Settings::watched_orgs` the first time it's empty. Kept as its own
/// function -- rather than inlined into `cycle` -- so the "only touch
/// disk when discovery actually changed something" shape is easy to
/// follow: it does not save `settings` itself, `cycle` does.
async fn discover_watched_orgs(api: &Client) -> crate::error::Result<Vec<String>> {
    let health = api.health().await?;
    let orgs = api.list_orgs().await?;
    Ok(query::merge_org_scopes(&health.login, orgs))
}

/// Whether org discovery should run this cycle. Driven purely by
/// `Settings::orgs_discovered`, not `watched_orgs.is_empty()`: an empty
/// list is ambiguous between "never discovered" and "the user unwatched
/// every org on purpose", and the old is_empty() check treated every
/// deliberate empty list as the former, silently repopulating it on the
/// very next poll. Discovery runs exactly once -- while this is `false`
/// -- and `cycle` sets the flag `true` the moment an attempt *completes*,
/// regardless of what it found, so a genuinely empty result also stops
/// it from running again.
///
/// Kept pure, like [`parse_anomaly`] and [`honors_wake`], so the decision
/// has unit coverage independent of `cycle`'s live `AppHandle`/network
/// calls.
pub fn should_discover_orgs(orgs_discovered: bool) -> bool {
    !orgs_discovered
}

/// Every distinct repo referenced across this cycle's raw results, in
/// first-seen order. Used to decide which repos' protected-branch cache
/// needs a refresh -- one `branches?protected=true` call per distinct
/// repo *in the inbox*, not per PR and not per watched org.
pub fn distinct_repos(results: &[Vec<PullRequest>]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for pr in results.iter().flatten() {
        if seen.insert(pr.repo.clone()) {
            out.push(pr.repo.clone());
        }
    }
    out
}

/// Protected-branch status changes rarely, so it's refetched at most
/// once a day per repo rather than every ~30s poll cycle -- fetching it
/// every cycle would add one HTTP call per distinct repo in the inbox,
/// on top of the search queries, for information that almost never
/// changes.
pub const PROTECTION_CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// Whether a repo's protected-branches cache entry needs a fresh fetch
/// this cycle. `None` means never fetched (a repo new to the inbox, or a
/// settings/state file predating this feature).
///
/// Kept pure, like [`parse_anomaly`] and [`honors_wake`], so the
/// decision has unit coverage independent of `cycle`'s live
/// `AppHandle`/network calls.
pub fn should_refresh_protection(cached_at: Option<i64>, now: i64) -> bool {
    match cached_at {
        None => true,
        Some(t) => now - t >= PROTECTION_CACHE_TTL_SECS,
    }
}

/// One poll cycle. Returns the assembled inbox plus any response-shape
/// anomaly, or an error the caller escalates on.
async fn cycle<H: PollHost>(rt: &PollRuntime<H>) -> crate::error::Result<InboxUpdate> {
    let dir = rt.dir.as_path();
    let (mut settings, _) = store::load_settings(dir);
    crate::auth::runner::set_gh_override(settings.gh_path.as_deref());

    let (token, _source) =
        crate::auth::resolve_token(&RealRunner).ok_or(crate::error::AppError::Unauthorized)?;
    let api = Client::new(token);

    // First-run population: rather than ever emitting an unscoped
    // "anyone can review" query -- which would match a large fraction of
    // every open PR on GitHub -- discovery runs once, gated on
    // `orgs_discovered` (see `should_discover_orgs`), and fills
    // `watched_orgs` in from `user/orgs` plus the viewer's own login.
    // Persisted immediately so later cycles (and the Settings screen) see
    // it without a second discovery call. The flag is set on any
    // *completed* attempt, even one that found nothing, which is what
    // lets the user unwatch every org afterwards and have that stick --
    // an empty `watched_orgs` no longer implies "go discover again". A
    // failed discovery (network error, etc.) is not escalated and leaves
    // the flag `false`, so the next cycle simply retries.
    if should_discover_orgs(settings.orgs_discovered) {
        if let Ok(discovered) = discover_watched_orgs(&api).await {
            if !discovered.is_empty() {
                settings.watched_orgs = discovered;
            }
            settings.orgs_discovered = true;
            let _ = store::save_settings(dir, &settings);
        }
    }

    let queries = query::build(&settings.rules, &settings.watched_orgs);
    let mut results = Vec::new();
    // Not fatal: keep polling and let the inbox update with whatever the
    // other queries found. The first anomaly is the one reported -- the
    // message names the failing shape, and repeating it per query would
    // only pad a 392px status strip.

    // Asked-for-you first, and deliberately: `inbox::assemble`'s dedupe
    // is first-occurrence-wins, so pre-flagging every PR in *this* batch
    // before pushing it means a PR present in both queries keeps the
    // flag with zero extra logic in `assemble`. See its own doc comment.
    let (mut asked_prs, asked_raw) = api.search_prs(&queries.asked_for_you).await?;
    let mut anomaly = parse_anomaly(asked_raw, asked_prs.len());
    for pr in &mut asked_prs {
        pr.asked_for_you = true;
    }
    results.push(asked_prs);

    for q in &queries.anyone {
        let (prs, raw_count) = api.search_prs(q).await?;
        if anomaly.is_none() {
            anomaly = parse_anomaly(raw_count, prs.len());
        }
        results.push(prs);
    }

    // Best-effort protected-branch refresh, one call per distinct repo
    // in this cycle's results whose cache is missing or stale (see
    // `should_refresh_protection`) -- not one per PR, and not every
    // cycle. Read here, before the state lock, so this network I/O never
    // blocks a Skip landing mid-cycle; a snapshot read is fine since
    // nothing else in the app writes `protected_branches`, so it can't
    // go stale between this read and the merge under the lock below. A
    // failed lookup (403/404/network/5xx) is silently dropped -- never
    // escalated, and never allowed to clobber a still-valid cache entry
    // -- `inbox::resolve_watched_branches` already falls through to the
    // global list for a repo with no cached entry, which is exactly the
    // right behavior for "couldn't check this time".
    let (snapshot_state, _) = store::load_state(dir);
    let now_for_protection = chrono::Utc::now().timestamp();
    let mut fresh_protection = std::collections::HashMap::new();
    for repo in distinct_repos(&results) {
        let cached_at = snapshot_state
            .protected_branches
            .get(&repo)
            .map(|c| c.checked_at);
        if !should_refresh_protection(cached_at, now_for_protection) {
            continue;
        }
        if let Ok(branches) = api.list_protected_branches(&repo).await {
            fresh_protection.insert(
                repo,
                store::ProtectedBranchesCache {
                    branches,
                    checked_at: now_for_protection,
                },
            );
        }
    }

    // Everything from here down is a load -> mutate -> save on state.json,
    // which `skip_pr` also performs. Without this lock a Skip landing
    // mid-cycle is read before the mutation and written back after it,
    // silently losing the mute. Deliberately taken *after* the network
    // calls, and the file loaded only now, so a Skip never blocks behind
    // an in-flight HTTP request.
    let _guard = rt.state_lock.lock().await;

    let (mut state, recovered) = store::load_state(dir);
    if recovered {
        // A corrupt state file was moved aside and defaults substituted:
        // every mute and every notified-marker is gone. Silent otherwise
        // -- the user would just see skipped PRs reappear and re-notify.
        rt.host.notify(
            "hi5 skip history was reset",
            "The state file was unreadable and has been backed up.",
        );
    }

    let now = chrono::Utc::now().timestamp();
    // Touched from the raw results, not the assembled list -- see
    // `seen_ids`. A muted PR is filtered out of `list` but is very much
    // still "seen", and pruning its mute would un-skip it.
    for id in seen_ids(&results) {
        state.touch(id, now);
    }

    // Merged in under the lock (not written back where it was fetched
    // above) so a concurrent Skip's read-modify-write can't race this --
    // same reasoning as `state.touch` above. Only ever adds/replaces
    // entries this cycle actually refreshed; a repo whose lookup failed
    // above keeps whatever was already cached for it.
    for (repo, entry) in fresh_protection {
        state.protected_branches.insert(repo, entry);
    }
    // Recorded for every repo with a known default branch this cycle --
    // not just the ones just (re)fetched -- so the Settings screen can
    // explain the "default branch" fallback tier for a repo whose
    // protection cache is still fresh from a previous cycle.
    for pr in results.iter().flatten() {
        if !pr.default_branch.is_empty() {
            state
                .repo_defaults
                .insert(pr.repo.clone(), pr.default_branch.clone());
        }
    }

    let list = inbox::assemble(results, &state, &settings.repos, &settings.branch_watch);

    if settings.notifications_enabled {
        let fresh = notify_diff::newly_notifiable(&list, &state);
        notify_diff::record(&mut state, &fresh);
        match notify_diff::banners(fresh) {
            notify_diff::Banners::Each(prs) => rt.host.notify_prs(&prs),
            notify_diff::Banners::Summary(n) => rt.host.notify(
                "hi5",
                &format!("{n} pull requests are waiting for a review"),
            ),
            notify_diff::Banners::Nothing => {}
        }
    }

    state.prune(now);
    let _ = store::save_state(dir, &state);

    Ok(InboxUpdate { prs: list, anomaly })
}

/// Escalation ladder for a 401: silently retry once, because gh may
/// have rotated the token. The retry needs no extra "re-read" step of
/// its own -- `cycle()` already calls `resolve_token`, which shells out
/// to `gh auth token` fresh on every call, so simply calling `cycle()`
/// again re-resolves a current token. Only bother the user if that
/// retry also fails.
async fn cycle_with_reauth<H: PollHost>(rt: &PollRuntime<H>) -> crate::error::Result<InboxUpdate> {
    match cycle(rt).await {
        Err(crate::error::AppError::Unauthorized) => cycle(rt).await,
        other => other,
    }
}

/// The poll loop. Runs until the process exits; `spawn` it on whatever
/// runtime the shell owns.
pub async fn run<H: PollHost>(rt: Arc<PollRuntime<H>>) {
    let (settings, _) = store::load_settings(&rt.dir);
    let mut backoff = Backoff::new(settings.poll_interval_secs);

    loop {
        // Signed out: no cycle, no badge, no banner — nothing at all
        // until a settings save (signing back in is one) wakes the loop.
        if store::load_settings(&rt.dir).0.signed_out {
            rt.wake.notified().await;
            continue;
        }

        let now = chrono::Utc::now().timestamp();
        let result = cycle_with_reauth(&rt).await;

        // Re-read the interval every cycle so a settings change takes
        // effect without a restart. Only `base_secs` is updated -- an
        // elevated `current_secs` from an ongoing backoff is left alone
        // and still resets to the *new* base on the next success.
        let (settings, _) = store::load_settings(&rt.dir);
        backoff.set_base(settings.poll_interval_secs);

        let delay = match &result {
            Ok(update) => {
                // Populate the cache before emitting: a panel opened
                // right after this event fires must see the fresh list,
                // not the previous one.
                *rt.cache.lock().await = update.prs.clone();

                rt.host.set_badge(Some(update.prs.len()));
                rt.host.emit(PollEvent::InboxUpdated(update.clone()));
                backoff.next_delay(None, now)
            }
            Err(e) if is_disconnecting_error(e) => {
                rt.host.set_badge(None);
                rt.host
                    .emit(PollEvent::AuthChanged(AuthState::Disconnected {
                        reason: "token expired or revoked".into(),
                    }));
                // Polling pauses: retrying a dead token accomplishes
                // nothing. The UI drives reconnection.
                Duration::from_secs(MAX_BACKOFF_SECS)
            }
            Err(e @ crate::error::AppError::RateLimited(reset_at)) => {
                // Distinct from `PollError`: the status strip needs to
                // say *why* nothing is refreshing, not just that
                // something failed.
                rt.host.emit(PollEvent::RateLimited(*reset_at));
                backoff.next_delay(Some(e), now)
            }
            Err(e) => {
                rt.host.emit(PollEvent::PollError(e.to_string()));
                backoff.next_delay(Some(e), now)
            }
        };

        // An explicit refresh or a settings change notifies `wake` to cut
        // the sleep short, rather than waiting out the full delay -- up
        // to 900s in a backed-off state. Except during a rate-limit
        // sleep: waking early there would re-issue the request before
        // GitHub's window reopens, worsening the exact situation the
        // user hitting refresh is trying to escape.
        if honors_wake(result.as_ref().err()) {
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = rt.wake.notified() => {}
            }
        } else {
            tokio::time::sleep(delay).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Author, CheckState};
    use crate::store::state::notified_key;
    use crate::store::{AppState, BranchWatch, RepoConfig};

    const DAY: i64 = 24 * 60 * 60;

    fn pr(id: &str, sha: &str) -> PullRequest {
        PullRequest {
            id: id.into(),
            number: 1,
            title: "t".into(),
            body: String::new(),
            url: String::new(),
            repo: "o/r".into(),
            author: Author {
                login: "a".into(),
                avatar_url: String::new(),
            },
            created_at: "2026-08-17T00:00:00Z".into(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            labels: vec![],
            head_sha: sha.into(),
            checks: CheckState::None,
            is_draft: false,
            base_ref_name: String::new(),
            default_branch: String::new(),
            asked_for_you: false,
        }
    }

    #[test]
    fn seen_ids_reports_every_raw_result_across_queries() {
        let results = vec![
            vec![pr("PR_1", "s"), pr("PR_2", "s")],
            vec![pr("PR_3", "s")],
        ];
        assert_eq!(seen_ids(&results), vec!["PR_1", "PR_2", "PR_3"]);
    }

    #[test]
    fn a_muted_pr_still_in_the_results_is_never_pruned() {
        // Regression: `cycle()` used to touch only the *assembled* list,
        // which `inbox::assemble` has already stripped muted PRs from --
        // so a muted PR's `last_seen` never advanced, `prune` deleted its
        // mute after 7 days, and it silently returned to the inbox and
        // re-notified without a single new commit. Spec §7 forbids that:
        // a mute lasts until the head SHA changes.
        //
        // This walks the poller's real per-cycle sequence (touch from raw
        // results -> assemble -> prune) once a day for longer than
        // PRUNE_AFTER_SECS, with the PR present in every raw response.
        let mut state = AppState::default();
        state.muted.insert("PR_1".into(), "sha_a".into());
        state.notified.insert(notified_key("PR_1", "sha_a"));
        state.touch("PR_1", 0);

        for day in 0..=8 {
            let now = day * DAY;
            let results = vec![vec![pr("PR_1", "sha_a")]];

            for id in seen_ids(&results) {
                state.touch(id, now);
            }
            let list = inbox::assemble(
                results,
                &state,
                &RepoConfig::default(),
                &BranchWatch::default(),
            );
            assert!(list.is_empty(), "day {day}: the mute must still hide it");

            state.prune(now);
        }

        assert_eq!(
            state.muted.get("PR_1").map(String::as_str),
            Some("sha_a"),
            "the mute must survive a week of cycles the PR was present for"
        );
        assert!(
            state.notified.contains(&notified_key("PR_1", "sha_a")),
            "the notified marker must survive too, or it re-notifies"
        );
    }

    #[test]
    fn a_muted_pr_that_actually_disappears_is_still_pruned() {
        // The other half of the contract: pruning must keep working for
        // PRs that genuinely stopped coming back (merged, closed).
        let mut state = AppState::default();
        state.muted.insert("PR_gone".into(), "sha_a".into());
        state.notified.insert(notified_key("PR_gone", "sha_a"));
        state.touch("PR_gone", 0);

        for day in 0..=8 {
            let now = day * DAY;
            let results: Vec<Vec<PullRequest>> = vec![vec![]];
            for id in seen_ids(&results) {
                state.touch(id, now);
            }
            state.prune(now);
        }

        assert!(!state.muted.contains_key("PR_gone"));
        assert!(!state.notified.contains(&notified_key("PR_gone", "sha_a")));
    }

    // Two tests lived here that asserted `InboxUpdate`'s JSON shape
    // (`{"prs":[],"anomaly":null}`), because it crossed a webview
    // boundary into `src/lib/types.ts` and a silent rename on either
    // side would have produced a permanently empty inbox. It crosses no
    // such boundary now -- it is handed to the shell in-process as a
    // `PollEvent::InboxUpdated`, where the compiler enforces what those
    // tests were checking by hand. They were deleted rather than
    // rewritten: asserting a struct literal survives being put in an
    // enum variant is a tautology, and keeping `serde::Serialize` on the
    // type to keep them compiling would have preserved a contract with a
    // consumer that no longer exists.
    //
    // What they were *really* protecting -- that a detected anomaly
    // reaches the user rather than being raced away by the very update
    // that found it -- is now structural rather than tested: there is no
    // separate anomaly event to reach for. `PollEvent` has four variants
    // and none of them carries an anomaly on its own, so the only way to
    // deliver one is inside the `InboxUpdated` that found it.
    // `parse_anomaly`'s own tests still cover *when* one is produced.

    #[test]
    fn success_holds_the_base_interval() {
        let mut b = Backoff::new(60);
        assert_eq!(b.next_delay(None, 0), Duration::from_secs(60));
        assert_eq!(b.next_delay(None, 0), Duration::from_secs(60));
    }

    #[test]
    fn failures_double_the_delay() {
        let mut b = Backoff::new(60);
        let e = AppError::GitHub("boom".into());
        assert_eq!(b.next_delay(Some(&e), 0), Duration::from_secs(120));
        assert_eq!(b.next_delay(Some(&e), 0), Duration::from_secs(240));
        assert_eq!(b.next_delay(Some(&e), 0), Duration::from_secs(480));
    }

    #[test]
    fn backoff_is_capped_at_fifteen_minutes() {
        let mut b = Backoff::new(60);
        let e = AppError::GitHub("boom".into());
        for _ in 0..20 {
            b.next_delay(Some(&e), 0);
        }
        assert_eq!(
            b.next_delay(Some(&e), 0),
            Duration::from_secs(MAX_BACKOFF_SECS)
        );
    }

    #[test]
    fn a_success_after_failures_resets_to_base() {
        let mut b = Backoff::new(60);
        let e = AppError::GitHub("boom".into());
        b.next_delay(Some(&e), 0);
        b.next_delay(Some(&e), 0);
        assert_eq!(b.next_delay(None, 0), Duration::from_secs(60));
    }

    #[test]
    fn rate_limit_sleeps_until_the_reset_timestamp() {
        let mut b = Backoff::new(60);
        let e = AppError::RateLimited(1000);
        assert_eq!(b.next_delay(Some(&e), 700), Duration::from_secs(300));
    }

    #[test]
    fn a_reset_in_the_past_still_waits_at_least_a_second() {
        let mut b = Backoff::new(60);
        let e = AppError::RateLimited(100);
        assert_eq!(b.next_delay(Some(&e), 500), Duration::from_secs(1));
    }

    #[test]
    fn set_base_does_not_disturb_an_in_flight_backoff() {
        let mut b = Backoff::new(60);
        let e = AppError::GitHub("boom".into());
        b.next_delay(Some(&e), 0); // 120
        b.next_delay(Some(&e), 0); // 240

        b.set_base(30);

        // The backed-off value is untouched by the interval change...
        assert_eq!(b.next_delay(Some(&e), 0), Duration::from_secs(480));
        // ...but a subsequent success adopts the new base.
        assert_eq!(b.next_delay(None, 0), Duration::from_secs(30));
    }

    #[test]
    fn nodes_returned_but_none_parsed_is_an_anomaly() {
        assert_eq!(
            parse_anomaly(23, 0),
            Some(
                "GitHub returned 23 PRs but none could be parsed \
                 — the API response shape may have changed"
                    .to_string()
            )
        );
    }

    #[test]
    fn nodes_returned_and_some_parsed_is_not_an_anomaly() {
        assert_eq!(parse_anomaly(5, 2), None);
    }

    #[test]
    fn zero_nodes_and_zero_parsed_is_the_healthy_empty_case() {
        // An empty inbox is normal and must never warn.
        assert_eq!(parse_anomaly(0, 0), None);
    }

    #[test]
    fn a_partial_parse_is_tolerated_by_design() {
        assert_eq!(parse_anomaly(10, 3), None);
    }

    #[test]
    fn success_honors_a_wake() {
        assert!(honors_wake(None));
    }

    #[test]
    fn a_generic_error_honors_a_wake() {
        let e = AppError::GitHub("boom".into());
        assert!(honors_wake(Some(&e)));
    }

    #[test]
    fn unauthorized_honors_a_wake() {
        assert!(honors_wake(Some(&AppError::Unauthorized)));
    }

    #[test]
    fn rate_limited_does_not_honor_a_wake() {
        // Waking early here would re-issue the request before GitHub's
        // window reopens -- exactly what the refresh button must not do.
        assert!(!honors_wake(Some(&AppError::RateLimited(1000))));
    }

    #[test]
    fn a_rejected_credential_disconnects_the_poller() {
        assert!(is_disconnecting_error(&AppError::Unauthorized));
    }

    #[test]
    fn discovery_runs_until_the_flag_says_it_has_completed_once() {
        assert!(should_discover_orgs(false));
        assert!(!should_discover_orgs(true));
    }

    #[test]
    fn a_non_credential_poll_failure_does_not_disconnect_the_poller() {
        // Regression guard: a 503, a rate limit, or a transport/offline
        // failure mid-cycle must never be classified the same as a
        // rejected credential -- that's the exact class of bug fixed for
        // `commands::get_auth_state`. This predicate is shared with that
        // fix (not a second copy of it) specifically so the poller and
        // `get_auth_state` can never disagree about what counts as a
        // rejection.
        assert!(!is_disconnecting_error(&AppError::GitHub(
            "http 503".into()
        )));
        assert!(!is_disconnecting_error(&AppError::RateLimited(1000)));
    }

    fn pr_in(id: &str, repo: &str) -> PullRequest {
        PullRequest {
            repo: repo.into(),
            ..pr(id, "s")
        }
    }

    #[test]
    fn distinct_repos_lists_each_repo_once_in_first_seen_order() {
        let results = vec![
            vec![pr_in("PR_1", "o/a"), pr_in("PR_2", "o/b")],
            vec![pr_in("PR_3", "o/a"), pr_in("PR_4", "o/c")],
        ];
        assert_eq!(distinct_repos(&results), vec!["o/a", "o/b", "o/c"]);
    }

    #[test]
    fn distinct_repos_of_empty_results_is_empty() {
        assert!(distinct_repos(&[]).is_empty());
    }

    #[test]
    fn protection_is_refreshed_when_never_cached() {
        assert!(should_refresh_protection(None, 1_000_000));
    }

    #[test]
    fn protection_is_not_refreshed_within_the_ttl() {
        let now = 1_000_000;
        assert!(!should_refresh_protection(
            Some(now - PROTECTION_CACHE_TTL_SECS + 1),
            now
        ));
    }

    #[test]
    fn protection_is_refreshed_once_the_ttl_has_elapsed() {
        let now = 1_000_000;
        assert!(should_refresh_protection(
            Some(now - PROTECTION_CACHE_TTL_SECS),
            now
        ));
        assert!(should_refresh_protection(
            Some(now - PROTECTION_CACHE_TTL_SECS - 1),
            now
        ));
    }
}
