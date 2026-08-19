use super::runner::CommandRunner;

#[derive(Clone, PartialEq)]
pub enum GhState {
    NotInstalled,
    NotAuthenticated,
    Ready { token: String },
}

/// Hand-written so a token can never reach a log line, a panic message,
/// or a `dbg!`. Deriving `Debug` here would put the raw credential one
/// stray `{:?}` away from disk.
impl std::fmt::Debug for GhState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "NotInstalled"),
            Self::NotAuthenticated => write!(f, "NotAuthenticated"),
            Self::Ready { .. } => write!(f, "Ready {{ token: <redacted> }}"),
        }
    }
}

/// Resolve the gh CLI's current credential. gh's output format is a de
/// facto interface, not a guaranteed one, so anything unexpected falls
/// through to NotAuthenticated rather than panicking.
pub fn detect(runner: &dyn CommandRunner) -> GhState {
    if runner.run("gh", &["--version"]).is_err() {
        return GhState::NotInstalled;
    }
    let Ok(out) = runner.run("gh", &["auth", "token"]) else {
        return GhState::NotAuthenticated;
    };
    if !out.status.success() {
        return GhState::NotAuthenticated;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return GhState::NotAuthenticated;
    }
    GhState::Ready { token }
}

/// The signed-in account name, read entirely from gh's local config via
/// `gh auth status` -- unlike `gh auth token` (which just echoes a
/// stored credential) or a `health()` call (which validates against
/// GitHub's `/user`), this never touches the network, so it keeps
/// working when GitHub itself is unreachable.
///
/// Used for the "connected but unverified" auth state: `health()`'s own
/// call to `/user` is exactly what failed there, so it can't supply the
/// login, but the user still has *a* credential worth naming.
///
/// gh's human-readable output is a de facto interface, not a guaranteed
/// one, so this parses defensively and returns `None` on anything that
/// doesn't match the expected `Logged in to <host> account <name> ...`
/// shape, rather than panicking or guessing.
pub fn login(runner: &dyn CommandRunner) -> Option<String> {
    let out = runner.run("gh", &["auth", "status"]).ok()?;
    // gh has written this line to stdout and, in older versions, to
    // stderr -- scan both rather than betting on either, and regardless
    // of exit status (`gh auth status` can exit non-zero for reasons
    // unrelated to the account line, e.g. a stale git-protocol config).
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_login(&combined)
}

fn parse_login(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if !line.contains("Logged in to") {
            continue;
        }
        let Some((_, after)) = line.split_once("account ") else {
            continue;
        };
        if let Some(name) = after.split_whitespace().next() {
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::runner::mock::MockRunner;

    #[test]
    fn reports_not_installed_when_the_binary_is_absent() {
        assert_eq!(detect(&MockRunner::missing_binary()), GhState::NotInstalled);
    }

    #[test]
    fn reports_not_authenticated_when_gh_exits_nonzero() {
        let r = MockRunner::default()
            .with("gh --version", 0, "gh version 2.88.0", "")
            .with("gh auth token", 1, "", "not logged into any hosts");
        assert_eq!(detect(&r), GhState::NotAuthenticated);
    }

    #[test]
    fn reports_not_authenticated_on_empty_stdout() {
        let r = MockRunner::default()
            .with("gh --version", 0, "gh version 2.88.0", "")
            .with("gh auth token", 0, "   \n", "");
        assert_eq!(detect(&r), GhState::NotAuthenticated);
    }

    #[test]
    fn returns_the_trimmed_token_when_authenticated() {
        let r = MockRunner::default()
            .with("gh --version", 0, "gh version 2.88.0", "")
            .with("gh auth token", 0, "gho_abc123\n", "");
        assert_eq!(
            detect(&r),
            GhState::Ready {
                token: "gho_abc123".into()
            }
        );
    }

    #[test]
    fn debug_output_does_not_leak_the_token() {
        let state = GhState::Ready {
            token: "gho_secret".into(),
        };
        let debug_str = format!("{:?}", state);
        assert!(!debug_str.contains("gho_secret"));
        assert!(debug_str.contains("<redacted>"));
    }

    #[test]
    fn parses_the_account_name_from_real_gh_auth_status_output() {
        // Captured verbatim from `gh auth status` (gh 2.88.0) on this
        // machine.
        let r = MockRunner::default().with(
            "gh auth status",
            0,
            "github.com\n  \u{2713} Logged in to github.com account jondot (keyring)\n  \
             - Active account: true\n  - Git operations protocol: ssh\n  \
             - Token: gho_************************************\n  \
             - Token scopes: 'gist', 'read:org', 'read:project', 'repo'\n",
            "",
        );
        assert_eq!(login(&r), Some("jondot".into()));
    }

    #[test]
    fn returns_none_when_logged_out() {
        let r = MockRunner::default().with(
            "gh auth status",
            1,
            "",
            "You are not logged into any GitHub hosts. Run gh auth login to authenticate.\n",
        );
        assert_eq!(login(&r), None);
    }

    #[test]
    fn returns_none_on_empty_output() {
        let r = MockRunner::default().with("gh auth status", 0, "", "");
        assert_eq!(login(&r), None);
    }

    #[test]
    fn returns_none_on_a_malformed_logged_in_line() {
        // The "account " marker present but no name following it --
        // must not panic, and must not silently invent a login.
        let r = MockRunner::default().with(
            "gh auth status",
            0,
            "  \u{2713} Logged in to github.com account \n",
            "",
        );
        assert_eq!(login(&r), None);
    }

    #[test]
    fn returns_none_when_gh_is_missing() {
        assert_eq!(login(&MockRunner::missing_binary()), None);
    }
}
