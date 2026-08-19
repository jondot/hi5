/// Fine-grained PATs carry no X-OAuth-Scopes header at all; classic
/// tokens and gh's OAuth tokens carry a comma-separated list.
#[derive(Debug, Clone, PartialEq)]
pub enum ScopeCheck {
    /// No scope header: a fine-grained token. Permissions can't be
    /// introspected, so proceed and let the API reject if inadequate.
    FineGrained,
    Classic {
        scopes: Vec<String>,
        adequate: bool,
    },
}

impl ScopeCheck {
    /// Whether this token can be expected to see private PRs.
    ///
    /// A fine-grained PAT is **never** reported as inadequate: it carries
    /// no scope header at all, so there is nothing to introspect, and it
    /// is the narrow path hi5 actually recommends (Pull requests: read &
    /// write). Blocking on it would block the good case.
    pub fn is_adequate(&self) -> bool {
        match self {
            ScopeCheck::FineGrained => true,
            ScopeCheck::Classic { adequate, .. } => *adequate,
        }
    }
}

pub fn parse(header: Option<&str>) -> ScopeCheck {
    match header {
        None => ScopeCheck::FineGrained,
        Some(h) if h.trim().is_empty() => ScopeCheck::FineGrained,
        Some(h) => {
            let scopes: Vec<String> = h
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let adequate = scopes.iter().any(|s| s == "repo");
            ScopeCheck::Classic { scopes, adequate }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_header_means_fine_grained_and_is_not_an_error() {
        assert_eq!(parse(None), ScopeCheck::FineGrained);
        assert_eq!(parse(Some("")), ScopeCheck::FineGrained);
    }

    #[test]
    fn recognises_the_real_gh_scope_string() {
        // Captured from `gh api /user -i` against gh 2.88.0.
        let c = parse(Some("gist, read:org, read:project, repo"));
        match c {
            ScopeCheck::Classic { scopes, adequate } => {
                assert!(adequate);
                assert_eq!(scopes.len(), 4);
                assert!(scopes.contains(&"repo".to_string()));
            }
            _ => panic!("expected classic"),
        }
    }

    #[test]
    fn flags_a_token_without_repo_as_inadequate() {
        let c = parse(Some("gist, read:org"));
        assert!(matches!(
            c,
            ScopeCheck::Classic {
                adequate: false,
                ..
            }
        ));
    }

    #[test]
    fn read_org_is_never_required() {
        let c = parse(Some("repo"));
        assert!(matches!(c, ScopeCheck::Classic { adequate: true, .. }));
    }

    #[test]
    fn a_fine_grained_token_is_never_reported_as_inadequate() {
        // No scope header means nothing to introspect, and fine-grained
        // is the narrow path hi5 recommends -- blocking here would block
        // the *better* token.
        assert!(parse(None).is_adequate());
        assert!(parse(Some("")).is_adequate());
    }

    #[test]
    fn is_adequate_follows_the_classic_repo_scope() {
        assert!(parse(Some("gist, read:org, read:project, repo")).is_adequate());
        assert!(!parse(Some("gist, read:org")).is_adequate());
    }
}
