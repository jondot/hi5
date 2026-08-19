pub mod gh;
pub mod health;
pub mod manual;
pub mod runner;

use crate::error::AppError;
use gh::GhState;
use runner::CommandRunner;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuthState {
    /// gh present and authenticated, or a stored manual token.
    Connected {
        login: String,
        source: String,
        scopes: Vec<String>,
        /// False only for a *classic* token missing `repo`. Such a token
        /// authenticates perfectly well, so this state stays `Connected`
        /// -- but GitHub search then returns public results only, giving
        /// a short inbox with no explanation. The frontend blocks on this
        /// with the `gh auth refresh -s repo` prompt spec §5.4 requires.
        /// Fine-grained tokens carry no scope header and are always
        /// adequate here; see `health::ScopeCheck::is_adequate`.
        scopes_adequate: bool,
        /// False when the credential was never actually confirmed
        /// against GitHub -- `health()`'s `GET /user` failed with
        /// something other than a 401 (a 5xx, rate limit, or a
        /// transport/offline failure), so the token *might* be fine but
        /// we couldn't check. `login` then comes from `gh auth status`
        /// (local, offline) rather than the `/user` response, and
        /// `scopes`/`scopes_adequate` are unknown rather than false --
        /// reporting them as inadequate would trigger the scope-upgrade
        /// screen for no reason, the same class of bug this field
        /// exists to fix. True whenever `health()` actually reached
        /// GitHub and got an answer.
        verified: bool,
    },
    GhNotInstalled {
        homebrew_available: bool,
    },
    GhNotAuthenticated,
    NeedsToken,
    /// Was connected, token stopped working.
    Disconnected {
        reason: String,
    },
    /// The user signed out of hi5 (`Settings::signed_out`). Not an
    /// error and not the CLI's state: a credential may well be there,
    /// and hi5 is choosing not to use it until asked.
    SignedOut,
}

/// Whether a `health()` failure means the stored credential itself is
/// bad, as opposed to GitHub simply being unreachable to verify it.
///
/// Only `AppError::Unauthorized` (a 401) means the credential was
/// actually rejected. Everything else -- a 5xx, `RateLimited`, or a
/// transport/offline failure via `AppError::Http`/`Io` -- means the
/// verification attempt itself failed, which says nothing about whether
/// the token is good. `get_auth_state` used to treat every `health()`
/// error alike, reporting "token rejected" for a GitHub outage and
/// pushing the user into `gh auth login` for nothing. This mirrors the
/// distinction `poller::spawn` already makes between
/// `Err(AppError::Unauthorized)` and every other error, kept as a pure
/// function (like `poller::parse_anomaly`/`honors_wake`) so the
/// classification has unit coverage independent of `get_auth_state`'s
/// real `Client`.
pub fn is_credential_rejected(err: &AppError) -> bool {
    matches!(err, AppError::Unauthorized)
}

/// Preference order: a manual token wins if one was explicitly stored,
/// otherwise gh. This lets a user override gh without signing gh out.
pub fn resolve_token(runner: &dyn CommandRunner) -> Option<(String, &'static str)> {
    if let Some(t) = manual::load_token() {
        return Some((t, "manual"));
    }
    match gh::detect(runner) {
        GhState::Ready { token } => Some((token, "gh")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_gh_not_installed_with_homebrew_available_camel_case() {
        let state = AuthState::GhNotInstalled {
            homebrew_available: true,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"homebrewAvailable\""));
        assert!(!json.contains("\"homebrew_available\""));
        assert!(json.contains("\"kind\":\"ghNotInstalled\""));
    }

    #[test]
    fn serializes_connected_with_camel_case_fields() {
        let state = AuthState::Connected {
            login: "octocat".into(),
            source: "gh".into(),
            scopes: vec!["repo".into()],
            scopes_adequate: true,
            verified: true,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"kind\":\"connected\""));
        assert!(json.contains("\"login\""));
        assert!(json.contains("\"source\""));
        assert!(json.contains("\"scopes\""));
        // src/lib/types.ts reads `scopesAdequate`; snake_case here would
        // read as `undefined` there and silently never block.
        assert!(json.contains("\"scopesAdequate\":true"));
        assert!(!json.contains("scopes_adequate"));
        assert!(json.contains("\"verified\":true"));
    }

    #[test]
    fn serializes_disconnected_with_camel_case_reason_field() {
        let state = AuthState::Disconnected {
            reason: "token expired".into(),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"kind\":\"disconnected\""));
        assert!(json.contains("\"reason\""));
    }

    #[test]
    fn all_kind_values_are_camel_case() {
        let states = [
            AuthState::Connected {
                login: "a".into(),
                source: "a".into(),
                scopes: vec![],
                scopes_adequate: true,
                verified: true,
            },
            AuthState::GhNotInstalled {
                homebrew_available: false,
            },
            AuthState::GhNotAuthenticated,
            AuthState::NeedsToken,
            AuthState::Disconnected { reason: "a".into() },
        ];

        let expected_kinds = [
            "connected",
            "ghNotInstalled",
            "ghNotAuthenticated",
            "needsToken",
            "disconnected",
        ];

        for (state, expected_kind) in states.iter().zip(expected_kinds.iter()) {
            let json = serde_json::to_string(state).unwrap();
            let expected = format!("\"kind\":\"{}\"", expected_kind);
            assert!(
                json.contains(&expected),
                "Expected {}, got {}",
                expected,
                json
            );
        }
    }

    #[test]
    fn only_unauthorized_means_the_credential_was_rejected() {
        assert!(is_credential_rejected(&AppError::Unauthorized));
    }

    #[test]
    fn a_generic_github_error_does_not_mean_the_credential_was_rejected() {
        // Reproduces the bug: GitHub's `/user` 503ing (or any other
        // non-401 status) must not read as "token rejected" -- the token
        // may be perfectly valid and GitHub is just unreachable.
        assert!(!is_credential_rejected(&AppError::GitHub(
            "http 503".into()
        )));
    }

    #[test]
    fn rate_limited_does_not_mean_the_credential_was_rejected() {
        assert!(!is_credential_rejected(&AppError::RateLimited(1000)));
    }

    #[test]
    fn a_transport_failure_does_not_mean_the_credential_was_rejected() {
        // The offline/transport half of "we couldn't verify" -- an Io
        // error (the other transport variant besides reqwest::Error)
        // must not read as a rejected credential either.
        let err: AppError = std::io::Error::new(std::io::ErrorKind::TimedOut, "offline").into();
        assert!(!is_credential_rejected(&err));
    }
}
