use crate::store::settings::Rules;

/// hi5's product model: the inbox is *not* your personal review queue --
/// it's every PR standing open for **anyone** to review, with PRs that
/// specifically name you highlighted rather than being the whole list.
/// That means two independent search queries, merged by `inbox::assemble`:
///
/// - "anyone can review": zero reviews yet, scoped per watched org. Must
///   never run unscoped -- `is:open is:pr review:none` alone matches a
///   large fraction of every open PR on GitHub.
/// - "asked for you": review specifically requested from you, global (no
///   org scoping needed -- it's already narrow).
///
/// Kept as two separate base strings (rather than one shared base with
/// clauses appended) because their qualifier sets don't overlap the way
/// the old single-rule-set design's did, and because keeping them
/// textually distinct makes the exact-string tests below easier to read.
const ANYONE_BASE: &str = "is:open is:pr review:none -author:@me";
const ASKED_BASE: &str = "is:open is:pr review-requested:@me";

/// The two query families for one poll cycle. Kept as named fields,
/// not a `Vec<String>`, so a caller (`poller::cycle`) can't accidentally
/// blend them and lose which batch is which -- that distinction is what
/// lets `askedForYou` be derived later purely from which query returned
/// a PR (see `inbox::assemble`).
pub struct Queries {
    /// Exactly one query, global, never empty.
    pub asked_for_you: String,
    /// One query per watched org. Empty when `watched_orgs` is empty --
    /// deliberately never falls back to an unscoped query.
    pub anyone: Vec<String>,
}

/// Build both query families for one poll cycle.
pub fn build(rules: &Rules, watched_orgs: &[String]) -> Queries {
    let mut asked_for_you = String::from(ASKED_BASE);
    if rules.hide_already_reviewed {
        asked_for_you.push_str(" -reviewed-by:@me");
    }
    if rules.hide_drafts {
        asked_for_you.push_str(" draft:false");
    }

    let anyone = watched_orgs
        .iter()
        .map(|org| {
            let mut q = String::from(ANYONE_BASE);
            q.push_str(" org:");
            q.push_str(org);
            if rules.hide_drafts {
                q.push_str(" draft:false");
            }
            q
        })
        .collect();

    Queries {
        asked_for_you,
        anyone,
    }
}

/// The candidate list of org-scoping toggles offered in Settings:
/// every org GitHub reports the viewer belongs to, plus the viewer's own
/// login so personal (non-org) repos remain reachable. Sorted and
/// deduplicated so it's stable to render and to diff against a saved
/// `watched_orgs` list.
pub fn merge_org_scopes(own_login: &str, orgs: Vec<String>) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> = orgs.into_iter().collect();
    if !own_login.is_empty() {
        set.insert(own_login.to_string());
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare() -> Rules {
        Rules {
            hide_already_reviewed: false,
            hide_drafts: false,
        }
    }

    #[test]
    fn default_rules_produce_the_exact_verified_asked_for_you_query() {
        // Verified live against the API (see final-fix-report.md):
        // "is:open is:pr review-requested:@me -reviewed-by:@me draft:false" -> 4
        let q = build(&Rules::default(), &[]);
        assert_eq!(
            q.asked_for_you,
            "is:open is:pr review-requested:@me -reviewed-by:@me draft:false"
        );
    }

    #[test]
    fn default_rules_produce_the_exact_verified_anyone_query_per_org() {
        // Verified live against the API (see final-fix-report.md):
        // "is:open is:pr org:acme-labs review:none -author:@me draft:false" -> 24
        let q = build(&Rules::default(), &["acme-labs".to_string()]);
        assert_eq!(q.anyone.len(), 1);
        assert_eq!(
            q.anyone[0],
            "is:open is:pr review:none -author:@me org:acme-labs draft:false"
        );
    }

    #[test]
    fn bare_rules_drop_both_modifiers() {
        let q = build(&bare(), &["acme".to_string()]);
        assert_eq!(q.asked_for_you, "is:open is:pr review-requested:@me");
        assert_eq!(
            q.anyone[0],
            "is:open is:pr review:none -author:@me org:acme"
        );
    }

    #[test]
    fn hide_already_reviewed_only_affects_the_asked_for_you_query() {
        // review:none already guarantees zero reviews, so appending
        // -reviewed-by:@me to the anyone query would be a no-op --
        // the toggle must not add it there.
        let mut r = bare();
        r.hide_already_reviewed = true;
        let q = build(&r, &["acme".to_string()]);
        assert!(q.asked_for_you.contains("-reviewed-by:@me"));
        assert!(!q.anyone[0].contains("-reviewed-by:@me"));
    }

    #[test]
    fn hide_drafts_affects_both_queries() {
        let mut r = bare();
        r.hide_drafts = true;
        let q = build(&r, &["acme".to_string()]);
        assert!(q.asked_for_you.ends_with("draft:false"));
        assert!(q.anyone[0].ends_with("draft:false"));
    }

    #[test]
    fn no_watched_orgs_emits_zero_anyone_queries() {
        // The critical safety property: never fall back to an unscoped
        // query, which would match a large fraction of every open PR on
        // GitHub. An empty scope list means "nothing to show", not
        // "show everything".
        let q = build(&Rules::default(), &[]);
        assert!(q.anyone.is_empty());
    }

    #[test]
    fn one_anyone_query_is_emitted_per_watched_org() {
        let orgs = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let q = build(&Rules::default(), &orgs);
        assert_eq!(q.anyone.len(), 3);
        for (org, query) in orgs.iter().zip(q.anyone.iter()) {
            assert!(query.contains(&format!("org:{org}")));
        }
    }

    #[test]
    fn the_asked_for_you_query_never_carries_an_org_qualifier() {
        // It's global by design -- narrow enough (review specifically
        // requested from you) that org scoping would only hide results.
        let q = build(&Rules::default(), &["acme".to_string()]);
        assert!(!q.asked_for_you.contains("org:"));
    }

    #[test]
    fn merge_org_scopes_includes_the_viewer_login_and_dedupes() {
        let merged = merge_org_scopes(
            "dipidi",
            vec![
                "acme".to_string(),
                "dipidi".to_string(),
                "acme-labs".to_string(),
            ],
        );
        assert_eq!(merged, vec!["acme", "acme-labs", "dipidi"]);
    }

    #[test]
    fn merge_org_scopes_tolerates_an_empty_login() {
        // Defensive: an empty login (a health check that returned
        // nothing useful) must not inject a bogus `org:` clause later.
        let merged = merge_org_scopes("", vec!["acme".to_string()]);
        assert_eq!(merged, vec!["acme"]);
    }
}
