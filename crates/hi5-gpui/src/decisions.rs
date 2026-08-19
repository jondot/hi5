//! The panel's decisions, as pure functions.
//!
//! `Hi5` is a view: constructing one needs a window, a running tokio
//! backend and a live menu-bar item, which is why its logic went
//! untested through two implementations of this app. Everything here is
//! the part that is *not* about drawing — which strip to show, whether
//! Approve may fire, what the badge counts — pulled out so it can be
//! exercised without any of that.
//!
//! Same pattern `hi5_core` already uses for `parse_anomaly`,
//! `honors_wake` and `is_credential_rejected`: keep the judgement in a
//! function with no context in its signature, and the context becomes a
//! thin caller.

use std::time::Duration;

use hi5_core::auth::AuthState;

/// The strip between the toolbar and the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Strip {
    Disconnected(String),
    RateLimited(i64),
    Stale(String),
}

/// A poll that has not landed in over three intervals — floored at two
/// minutes for a very fast interval — is worth flagging. Otherwise a
/// stuck poller looks exactly like "nothing new to review", and the user
/// has no reason to distrust it.
const MIN_STALE: Duration = Duration::from_secs(120);
const STALE_MULTIPLIER: u32 = 3;

pub fn stale_after(poll_interval_secs: u64) -> Duration {
    Duration::from_secs(poll_interval_secs)
        .saturating_mul(STALE_MULTIPLIER)
        .max(MIN_STALE)
}

/// Which strip to show, most-actionable first.
///
/// A broken connection explains everything else; a rate limit explains
/// why refresh looks like it is doing nothing and carries a concrete
/// resolution time; a poll error is a one-off with real detail;
/// staleness is the fallback when none of those fired.
pub fn strip(
    auth: Option<&AuthState>,
    rate_limited_until: Option<i64>,
    poll_error: Option<&str>,
    verified_by_poll: bool,
    since_last_poll: Option<Duration>,
    poll_interval_secs: u64,
) -> Option<Strip> {
    if matches!(auth, Some(AuthState::Disconnected { .. })) {
        return Some(Strip::Disconnected(
            "Lost access to GitHub — showing your last inbox".into(),
        ));
    }
    if let Some(reset) = rate_limited_until {
        return Some(Strip::RateLimited(reset));
    }
    if let Some(err) = poll_error {
        return Some(Strip::Disconnected(crate::ui::format::humanize_poll_error(
            err,
        )));
    }
    // Connected but never actually confirmed — the health check failed
    // with something that was not a 401, so the token may be perfectly
    // fine. The user is let through to the inbox rather than blocked,
    // but is owed an honest "couldn't check" rather than a silent
    // pretence that it was verified.
    //
    // A successful poll cycle overrides that: GitHub would not run an
    // authenticated search and hand back private review requests for a
    // rejected credential, so the banner must not contradict real pull
    // requests sitting right below it.
    if let Some(AuthState::Connected {
        verified: false, ..
    }) = auth
    {
        if !verified_by_poll {
            return Some(Strip::Disconnected(
                "Couldn't verify your GitHub token — GitHub may be unreachable".into(),
            ));
        }
    }
    let elapsed = since_last_poll?;
    if elapsed > stale_after(poll_interval_secs) {
        let mins = (elapsed.as_secs() / 60).max(1);
        return Some(Strip::Stale(format!(
            "Last updated {mins} minute{} ago",
            if mins == 1 { "" } else { "s" }
        )));
    }
    None
}

/// Whether Approve may fire.
///
/// Two guards, both load-bearing: it stays inert for 250ms after the
/// detail view appears, so a fast double-click on a list row cannot
/// carry through into a public review; and `busy` is set synchronously
/// before the request leaves, so a held ⌘↵ dispatching the action
/// repeatedly is refused after the first.
pub fn may_approve(on_detail: bool, armed: bool, busy: bool) -> bool {
    on_detail && armed && !busy
}

/// What the menu-bar item says beside the hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    /// So many to review — and zero is its own gesture (🤘), because an
    /// empty queue is the good news this app exists to deliver.
    Count(usize),
    /// Disconnected: `!`, so a broken connection and an empty queue can
    /// never look the same.
    Broken,
    /// Signed out: the hand alone. hi5 has nothing to say and no reason
    /// to alarm.
    Quiet,
}

/// What the menu-bar badge shows.
///
/// The count is of the *visible* list — repo focus applied, then the
/// active segment — not the account-wide total, or the menu bar
/// contradicts the window it opens.
pub fn badge(auth: Option<&AuthState>, visible: usize) -> Badge {
    match auth {
        Some(AuthState::Disconnected { .. }) => Badge::Broken,
        Some(AuthState::SignedOut) => Badge::Quiet,
        _ => Badge::Count(visible),
    }
}

/// Whether to show the auth screen instead of the inbox.
///
/// Three conditions: any non-connected state; the very first run even
/// when already connected (the "Signed in as *login*" confirmation); and
/// a classic token missing `repo`, which authenticates happily and then
/// quietly returns public results only — a short inbox with no
/// explanation.
pub fn needs_auth(auth: Option<&AuthState>, completed_first_run: bool) -> bool {
    match auth {
        None => false,
        Some(AuthState::Connected {
            scopes_adequate, ..
        }) => !scopes_adequate || !completed_first_run,
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(verified: bool, scopes_adequate: bool) -> AuthState {
        AuthState::Connected {
            login: "someone".into(),
            source: "gh".into(),
            scopes: vec!["repo".into()],
            scopes_adequate,
            verified,
        }
    }

    fn disconnected() -> AuthState {
        AuthState::Disconnected {
            reason: "token expired or revoked".into(),
        }
    }

    // ---- strip ordering ----

    #[test]
    fn a_broken_connection_outranks_everything_else() {
        // It explains the rate limit, the poll error and the staleness,
        // so showing any of those instead would be answering a question
        // the user has not got to yet.
        let s = strip(
            Some(&disconnected()),
            Some(1_000),
            Some("github api error: http 503"),
            false,
            Some(Duration::from_secs(9_999)),
            30,
        );
        assert!(matches!(s, Some(Strip::Disconnected(m)) if m.contains("Lost access")));
    }

    #[test]
    fn a_rate_limit_outranks_a_poll_error_and_carries_its_reset_time() {
        // The strip has to say *why* nothing is refreshing, and when it
        // will — a generic failure message would leave the user hitting
        // refresh into a wall.
        let s = strip(
            Some(&connected(true, true)),
            Some(1_755_000_000),
            Some("github api error: http 503"),
            true,
            None,
            30,
        );
        assert_eq!(s, Some(Strip::RateLimited(1_755_000_000)));
    }

    #[test]
    fn a_poll_error_is_humanised_not_echoed() {
        let s = strip(
            Some(&connected(true, true)),
            None,
            Some("github api error: http 503"),
            true,
            None,
            30,
        );
        let Some(Strip::Disconnected(msg)) = s else {
            panic!("expected a disconnected strip, got {s:?}")
        };
        assert!(msg.contains("GitHub is having trouble"), "{msg}");
        assert!(!msg.contains("503"), "{msg}");
    }

    #[test]
    fn an_unverified_token_says_so_rather_than_pretending() {
        // The health check failed with something that was not a 401, so
        // the token may be fine — but claiming it was verified would be
        // a lie, and this app has shipped a silent-empty-inbox bug
        // before.
        let s = strip(Some(&connected(false, true)), None, None, false, None, 30);
        assert!(matches!(s, Some(Strip::Disconnected(m)) if m.contains("Couldn't verify")));
    }

    #[test]
    fn a_successful_poll_overrides_the_unverified_banner() {
        // GitHub would not run an authenticated search and hand back
        // private review requests for a rejected credential, so the
        // banner must not contradict the pull requests below it.
        let s = strip(Some(&connected(false, true)), None, None, true, None, 30);
        assert_eq!(s, None);
    }

    #[test]
    fn staleness_is_the_last_resort_and_needs_three_intervals() {
        let ok = strip(
            Some(&connected(true, true)),
            None,
            None,
            true,
            Some(Duration::from_secs(150)),
            60,
        );
        assert_eq!(ok, None, "150s is inside 3x60s");

        let stale = strip(
            Some(&connected(true, true)),
            None,
            None,
            true,
            Some(Duration::from_secs(200)),
            60,
        );
        assert!(matches!(stale, Some(Strip::Stale(m)) if m.contains("3 minutes")));
    }

    #[test]
    fn a_very_fast_interval_still_gets_a_two_minute_floor() {
        // Three times a five-second interval is fifteen seconds, and a
        // banner every fifteen seconds would be noise, not a signal.
        assert_eq!(stale_after(5), Duration::from_secs(120));
        assert_eq!(stale_after(60), Duration::from_secs(180));
    }

    #[test]
    fn nothing_is_shown_before_the_first_poll_lands() {
        // No elapsed time means no cycle has completed yet, which is not
        // the same as a stale one.
        assert_eq!(
            strip(Some(&connected(true, true)), None, None, true, None, 30),
            None
        );
    }

    // ---- approve guards ----

    #[test]
    fn approve_is_refused_until_armed() {
        assert!(!may_approve(true, false, false));
        assert!(may_approve(true, true, false));
    }

    #[test]
    fn approve_is_refused_while_a_request_is_in_flight() {
        // This is what stops a held ⌘↵ posting a second review: `busy`
        // is set synchronously before the first request leaves.
        assert!(!may_approve(true, true, true));
    }

    #[test]
    fn approve_is_refused_anywhere_but_the_detail_screen() {
        // The button exists only there, but the *action* is bound
        // globally, so holding ⌘↵ on the inbox dispatches it too.
        assert!(!may_approve(false, true, false));
    }

    // ---- badge ----

    #[test]
    fn the_badge_counts_what_is_visible() {
        assert_eq!(badge(Some(&connected(true, true)), 17), Badge::Count(17));
    }

    #[test]
    fn a_broken_connection_badges_differently_from_an_empty_queue() {
        // Both would otherwise render as nothing in the menu bar.
        assert_eq!(badge(Some(&disconnected()), 0), Badge::Broken);
        assert_eq!(badge(Some(&connected(true, true)), 0), Badge::Count(0));
    }

    #[test]
    fn signed_out_is_quiet_not_broken() {
        assert_eq!(badge(Some(&AuthState::SignedOut), 3), Badge::Quiet);
        assert!(needs_auth(Some(&AuthState::SignedOut), true));
    }

    // ---- auth gating ----

    #[test]
    fn a_connected_account_past_first_run_goes_straight_to_the_inbox() {
        assert!(!needs_auth(Some(&connected(true, true)), true));
    }

    #[test]
    fn the_welcome_screen_shows_once_and_only_once() {
        assert!(needs_auth(Some(&connected(true, true)), false));
        assert!(!needs_auth(Some(&connected(true, true)), true));
    }

    #[test]
    fn a_token_that_cannot_see_private_repos_blocks() {
        // It authenticates happily and then returns public results only,
        // which is a short inbox with no explanation — the exact failure
        // this gate exists for.
        assert!(needs_auth(Some(&connected(true, false)), true));
    }

    #[test]
    fn every_non_connected_state_blocks() {
        for state in [
            disconnected(),
            AuthState::GhNotAuthenticated,
            AuthState::NeedsToken,
            AuthState::GhNotInstalled {
                homebrew_available: true,
            },
        ] {
            assert!(needs_auth(Some(&state), true), "{state:?}");
        }
    }

    #[test]
    fn an_unknown_auth_state_shows_nothing_rather_than_the_auth_screen() {
        // `None` is "the check has not come back yet". Showing the
        // reconnect screen for that would flash it on every launch.
        assert!(!needs_auth(None, true));
    }
}
