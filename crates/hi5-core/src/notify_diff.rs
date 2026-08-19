use crate::github::PullRequest;
use crate::store::state::notified_key;
use crate::store::AppState;

/// A PR is notifiable once per (id, head_sha). Re-notifying after a new
/// commit is intentional: something changed and it's back in the inbox.
pub fn newly_notifiable<'a>(inbox: &'a [PullRequest], state: &AppState) -> Vec<&'a PullRequest> {
    inbox
        .iter()
        .filter(|pr| !state.notified.contains(&notified_key(&pr.id, &pr.head_sha)))
        .collect()
}

/// Above this many newly-notifiable PRs in one cycle, they are one
/// banner, not one each. A cycle that turns up more than a handful of
/// new PRs at once is not "something just happened" — it is a first
/// run, a sign-in, or the first cycle after a long absence, and the
/// first release posted 218 banners on first launch for exactly that
/// reason. Six is about what a person can read as they arrive.
pub const BURST: usize = 6;

/// What to post for a cycle's newly-notifiable PRs.
#[derive(Debug, PartialEq)]
pub enum Banners<'a> {
    /// One per PR, author and title.
    Each(Vec<&'a PullRequest>),
    /// One in total, saying how many.
    Summary(usize),
    Nothing,
}

pub fn banners(fresh: Vec<&PullRequest>) -> Banners<'_> {
    match fresh.len() {
        0 => Banners::Nothing,
        n if n > BURST => Banners::Summary(n),
        _ => Banners::Each(fresh),
    }
}

pub fn record(state: &mut AppState, prs: &[&PullRequest]) {
    for pr in prs {
        state.notified.insert(notified_key(&pr.id, &pr.head_sha));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Author, CheckState};

    #[test]
    fn a_burst_of_new_pull_requests_is_one_banner_not_one_each() {
        let many: Vec<PullRequest> = (0..40).map(|i| pr(&format!("PR_{i}"), "sha")).collect();
        assert_eq!(banners(many.iter().collect()), Banners::Summary(40));
        let few: Vec<PullRequest> = (0..BURST).map(|i| pr(&format!("PR_{i}"), "sha")).collect();
        assert!(matches!(banners(few.iter().collect()), Banners::Each(v) if v.len() == BURST));
        assert_eq!(banners(Vec::new()), Banners::Nothing);
    }

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
    fn every_pr_is_notifiable_on_a_cold_start() {
        let inbox = vec![pr("PR_1", "a"), pr("PR_2", "b")];
        assert_eq!(newly_notifiable(&inbox, &AppState::default()).len(), 2);
    }

    #[test]
    fn a_pr_is_not_notified_twice_across_poll_cycles() {
        let inbox = vec![pr("PR_1", "a")];
        let mut state = AppState::default();

        let first = newly_notifiable(&inbox, &state);
        assert_eq!(first.len(), 1);
        record(&mut state, &first);

        assert!(newly_notifiable(&inbox, &state).is_empty());
    }

    #[test]
    fn a_new_commit_makes_a_pr_notifiable_again() {
        let mut state = AppState::default();
        let old = vec![pr("PR_1", "sha_a")];
        let fresh = newly_notifiable(&old, &state);
        record(&mut state, &fresh);

        let updated = vec![pr("PR_1", "sha_b")];
        assert_eq!(newly_notifiable(&updated, &state).len(), 1);
    }
}
