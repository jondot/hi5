//! The inbox's two view filters, in one place.
//!
//! This lives in the domain rather than in a screen because two callers
//! need the same answer and must not disagree: the list, which renders
//! `visible`, and the menu-bar badge, which counts it. When the badge
//! was computed separately it advertised the account-wide total while
//! the panel showed a filtered subset — the menu bar claiming 220 next
//! to a list of 17.

use crate::github::PullRequest;

/// Which slice of the queue the segmented control is showing.
///
/// A filter over one list, not a second information architecture: both
/// scopes keep the repo grouping, the sticky headers and the backend's
/// ordering, so switching never reshuffles anything under the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    All,
    ForYou,
}

pub struct InboxView<'a> {
    /// Everything left after the repo focus filter — what "All" counts.
    pub scoped: Vec<&'a PullRequest>,
    /// `scoped` narrowed by the scope — what the list renders.
    pub visible: Vec<&'a PullRequest>,
    /// How many of `scoped` name you specifically — what "For you"
    /// counts.
    pub for_you: usize,
}

/// Apply the repo focus, then the scope.
///
/// The order is the part worth protecting: focus first, so both counts
/// in the segmented control describe the repos you are currently looking
/// at rather than the whole account. An empty `focus` means no focus at
/// all — every repo shows.
pub fn inbox_view<'a>(prs: &'a [PullRequest], focus: &[String], scope: Scope) -> InboxView<'a> {
    let scoped: Vec<&PullRequest> = if focus.is_empty() {
        prs.iter().collect()
    } else {
        prs.iter().filter(|p| focus.contains(&p.repo)).collect()
    };
    let for_you = scoped.iter().filter(|p| p.asked_for_you).count();
    let visible = match scope {
        Scope::All => scoped.clone(),
        Scope::ForYou => scoped.iter().copied().filter(|p| p.asked_for_you).collect(),
    };
    InboxView {
        scoped,
        visible,
        for_you,
    }
}

/// The repos to offer in the filter menu, with a count each.
///
/// Counted from the *unfocused* list: the menu has to keep showing what
/// you would get back by widening the filter, not just what is left
/// inside it. A focused repo whose PRs have all been reviewed drops out
/// of `prs` entirely, so it is re-added at zero — without that it
/// vanishes from the menu while still filtering the list, leaving an
/// inbox stuck at empty with the way out invisible.
pub fn repo_counts(prs: &[PullRequest], focus: &[String]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for pr in prs {
        *counts.entry(pr.repo.clone()).or_insert(0) += 1;
    }
    for repo in focus {
        counts.entry(repo.clone()).or_insert(0);
    }
    let mut counts: Vec<(String, usize)> = counts.into_iter().collect();
    // Case-insensitively, so `FieldBytes-Inc` sits among the f's rather
    // than ahead of every lowercase org. A plain byte sort puts every
    // capitalised owner in a block at the top of the menu, which reads
    // as a second, unexplained grouping.
    counts.sort_by(|a, b| {
        a.0.to_lowercase()
            .cmp(&b.0.to_lowercase())
            .then_with(|| a.0.cmp(&b.0))
    });
    counts
}

/// The visible list grouped by repo, preserving the backend's ordering
/// within each group and the order the repos first appear.
pub fn group_by_repo<'a>(visible: &[&'a PullRequest]) -> Vec<(String, Vec<&'a PullRequest>)> {
    let mut groups: Vec<(String, Vec<&PullRequest>)> = Vec::new();
    for pr in visible {
        match groups.iter_mut().find(|(repo, _)| repo == &pr.repo) {
            Some((_, list)) => list.push(pr),
            None => groups.push((pr.repo.clone(), vec![pr])),
        }
    }
    groups
}

/// The visible list in the order it is actually drawn: grouped by repo,
/// then flattened.
///
/// Keyboard selection has to index *this*, not `visible`. The backend
/// orders the queue by diff size, so grouping reshuffles it — and an
/// arrow key that walked the pre-group order would jump around the
/// screen, selecting a row eight groups further down.
pub fn display_order<'a>(visible: &[&'a PullRequest]) -> Vec<&'a PullRequest> {
    group_by_repo(visible)
        .into_iter()
        .flat_map(|(_, prs)| prs)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Author, CheckState};

    fn pr(id: &str, repo: &str, asked_for_you: bool) -> PullRequest {
        PullRequest {
            id: id.into(),
            number: 1,
            title: id.into(),
            body: String::new(),
            url: String::new(),
            repo: repo.into(),
            author: Author {
                login: "someone".into(),
                avatar_url: String::new(),
            },
            created_at: "2026-08-01T00:00:00Z".into(),
            additions: 1,
            deletions: 0,
            changed_files: 1,
            labels: vec![],
            head_sha: "abc".into(),
            checks: CheckState::None,
            is_draft: false,
            base_ref_name: "main".into(),
            default_branch: "main".into(),
            asked_for_you,
        }
    }

    fn fixture() -> Vec<PullRequest> {
        vec![
            pr("a", "org/one", true),
            pr("b", "org/one", false),
            pr("c", "org/two", false),
            pr("d", "org/three", true),
        ]
    }

    fn ids(prs: &[&PullRequest]) -> Vec<String> {
        prs.iter().map(|p| p.id.clone()).collect()
    }

    #[test]
    fn passes_everything_through_with_no_focus_and_the_all_scope() {
        let prs = fixture();
        let v = inbox_view(&prs, &[], Scope::All);
        assert_eq!(v.visible.len(), 4);
        assert_eq!(v.scoped.len(), 4);
        assert_eq!(v.for_you, 2);
    }

    #[test]
    fn narrows_to_the_focused_repo() {
        let prs = fixture();
        let v = inbox_view(&prs, &["org/one".into()], Scope::All);
        assert_eq!(ids(&v.visible), ["a", "b"]);
    }

    #[test]
    fn focuses_on_several_repos_at_once() {
        let prs = fixture();
        let v = inbox_view(&prs, &["org/one".into(), "org/two".into()], Scope::All);
        assert_eq!(ids(&v.visible), ["a", "b", "c"]);
    }

    #[test]
    fn counts_for_you_within_the_focus_not_across_the_account() {
        // "d" is flagged for you but lives outside the focused repo, so
        // this must be 1 and not 2 -- it is the number the segmented
        // control and the menu-bar badge both read.
        let prs = fixture();
        let v = inbox_view(&prs, &["org/one".into()], Scope::All);
        assert_eq!(v.for_you, 1);
    }

    #[test]
    fn applies_the_scope_after_the_focus() {
        let prs = fixture();
        let v = inbox_view(&prs, &["org/one".into()], Scope::ForYou);
        assert_eq!(ids(&v.visible), ["a"]);
        // `scoped` stays the pre-scope list -- it is what "All" counts.
        assert_eq!(v.scoped.len(), 2);
    }

    #[test]
    fn yields_an_empty_view_for_a_focus_that_matches_nothing() {
        let prs = fixture();
        let v = inbox_view(&prs, &["org/gone".into()], Scope::All);
        assert!(v.visible.is_empty());
        assert_eq!(v.for_you, 0);
    }

    #[test]
    fn repo_counts_sort_case_insensitively() {
        let prs = vec![
            pr("a", "Zeta/one", false),
            pr("b", "alpha/two", false),
            pr("c", "Beta/three", false),
        ];
        let names: Vec<String> = repo_counts(&prs, &[]).into_iter().map(|(r, _)| r).collect();
        assert_eq!(names, ["alpha/two", "Beta/three", "Zeta/one"]);
    }

    #[test]
    fn repo_counts_come_from_the_unfocused_list() {
        let prs = fixture();
        let counts = repo_counts(&prs, &["org/one".into()]);
        assert_eq!(
            counts,
            vec![
                ("org/one".to_string(), 2),
                ("org/three".to_string(), 1),
                ("org/two".to_string(), 1),
            ]
        );
    }

    #[test]
    fn a_focused_repo_with_nothing_left_stays_listed_at_zero() {
        // Otherwise it disappears from the menu while still filtering the
        // list, and the only way out of an empty inbox is invisible.
        let prs = fixture();
        let counts = repo_counts(&prs, &["org/vanished".into()]);
        assert!(counts.contains(&("org/vanished".to_string(), 0)));
    }

    #[test]
    fn display_order_is_the_grouped_order_not_the_backend_order() {
        // The backend hands back a, b (org/one), c (org/two), d
        // (org/three) -- already grouped here -- but interleave them and
        // the display order must still be group-major.
        let prs = vec![
            pr("a", "org/one", false),
            pr("c", "org/two", false),
            pr("b", "org/one", false),
        ];
        let v = inbox_view(&prs, &[], Scope::All);
        assert_eq!(ids(&display_order(&v.visible)), ["a", "b", "c"]);
    }

    #[test]
    fn grouping_keeps_first_appearance_order_and_within_group_order() {
        let prs = fixture();
        let v = inbox_view(&prs, &[], Scope::All);
        let groups = group_by_repo(&v.visible);
        let names: Vec<&str> = groups.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(names, ["org/one", "org/two", "org/three"]);
        assert_eq!(ids(&groups[0].1), ["a", "b"]);
    }
}
