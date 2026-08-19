use crate::github::PullRequest;
use crate::store::{AppState, BranchWatch, RepoConfig};
use std::collections::HashSet;

/// Resolves "the branches we care about" for one repo, in strict
/// priority order. A PR merging one feature branch into another --
/// where neither is protected and neither is the default -- gates
/// nothing, and is noise in a shared review queue (see `BranchWatch`'s
/// doc comment for the live example this exists for).
///
/// 1. `per_repo_override` -- a user-set override for this specific repo,
///    wins outright when non-empty.
/// 2. `detected` -- the repo's actual protected branches, from
///    `GET /repos/{owner}/{repo}/branches?protected=true` (cached; see
///    `poller::should_refresh_protection`). Trusted over the global list
///    when non-empty, since it's the real answer for *this* repo.
/// 3. `global_list` -- the user-editable fallback (default
///    `["main", "master", "develop"]`), used when detection came back
///    empty -- which is genuinely ambiguous between "no protected
///    branches" and "no permission to see them" (verified live against
///    `rusty-ferris-club/rust-starter`; there is no way to tell the two
///    apart from the API response alone).
/// 4. `default_branch` -- the repo's own default branch, last resort,
///    only reached if the user has also emptied the global list.
///
/// If every tier is empty (global list emptied *and* no default branch
/// known) this returns an empty `Vec` -- callers must treat that as
/// "nothing resolved, so don't filter" rather than "watch nothing, hide
/// every PR in this repo". The two are easy to conflate and the
/// consequence of getting it wrong is a silently empty inbox, so this is
/// pinned by its own test below (`unresolvable_tiers_yield_an_empty_list_not_a_hide_all_signal`)
/// and by `inbox::assemble`'s `an_unresolvable_watch_set_does_not_hide_the_pr`.
pub fn resolve_watched_branches(
    per_repo_override: Option<&[String]>,
    detected: &[String],
    global_list: &[String],
    default_branch: &str,
) -> Vec<String> {
    if let Some(overrides) = per_repo_override {
        if !overrides.is_empty() {
            return overrides.to_vec();
        }
    }
    if !detected.is_empty() {
        return detected.to_vec();
    }
    if !global_list.is_empty() {
        return global_list.to_vec();
    }
    if !default_branch.is_empty() {
        return vec![default_branch.to_string()];
    }
    Vec::new()
}

/// Mirrors `resolve_watched_branches`'s tier order to explain, in the
/// Settings screen, *where* a repo's resolved list came from -- the
/// safeguard for silent hiding the developer explicitly asked for. Kept
/// as a sibling function (not folded into `resolve_watched_branches`'s
/// return value) so that function's signature stays exactly the pure
/// `Vec<String>` shape used by the actual filter in `assemble`.
pub fn branch_watch_source(
    per_repo_override: Option<&[String]>,
    detected: &[String],
    global_list: &[String],
    default_branch: &str,
) -> &'static str {
    if per_repo_override.is_some_and(|o| !o.is_empty()) {
        "override"
    } else if !detected.is_empty() {
        "detected"
    } else if !global_list.is_empty() {
        "global"
    } else if !default_branch.is_empty() {
        "default"
    } else {
        "unfiltered"
    }
}

/// Whether `pr` should be dropped for targeting a branch nobody asked to
/// watch. `false` whenever the PR's own `base_ref_name` is unknown (an
/// empty string -- see `models::node_to_pr`'s comment on why that field
/// is never required) or `watched` came back empty from
/// `resolve_watched_branches` -- both are "can't judge this", and the
/// safe default is to show the PR, not hide it.
fn is_unwatched_branch(pr: &PullRequest, watched: &[String]) -> bool {
    if pr.base_ref_name.is_empty() || watched.is_empty() {
        return false;
    }
    !watched.iter().any(|b| b == &pr.base_ref_name)
}

/// Combine per-query results into the list the user sees.
///
/// Order within the input is preserved; the first occurrence of a PR
/// wins the dedupe. The caller (`poller::cycle`) relies on that: passing
/// the "asked for you" batch first, with every PR in it pre-flagged
/// `asked_for_you = true`, means a PR present in both queries keeps the
/// flag with no extra logic here -- the "anyone can review" copy of the
/// same PR is simply the duplicate that loses.
///
/// The result is then sorted `asked_for_you` PRs first (hi5's opportunistic
/// premise doesn't override a direct ask), then, within each of those two
/// tiers, by diff size ascending -- `additions + deletions` -- so a
/// `+65 -53` PR sits above a `+896 -449` one and whoever's free can clear
/// the most PRs in the time they have. Ties (equal size, most commonly
/// two 0-line PRs) fall back to the PR's GitHub node id purely for a
/// deterministic order: relying on "the sort is stable, so equal keys
/// keep their input order" is not enough here, because the *input*
/// order -- whatever GitHub's search API returned this cycle -- is not
/// itself guaranteed stable across polls. Without an explicit tie-break,
/// two equal-size PRs could swap places between polls with nothing in
/// the diff to explain why. The frontend groups by repo in first-seen
/// order (a plain JS object accumulation, see Inbox.tsx), so this full
/// ordering -- flagged-first, then size, then id -- is what puts the
/// right PR at the top of *its own* repo group without the frontend
/// needing to sort anything itself.
pub fn assemble(
    results: Vec<Vec<PullRequest>>,
    state: &AppState,
    repos: &RepoConfig,
    branch_watch: &BranchWatch,
) -> Vec<PullRequest> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    for batch in results {
        for pr in batch {
            if !seen.insert(pr.id.clone()) {
                continue; // already included from an earlier query
            }
            if repos.muted.contains(&pr.repo) {
                continue;
            }
            // A muted PR returns the moment its head commit changes.
            if state
                .muted
                .get(&pr.id)
                .is_some_and(|sha| sha == &pr.head_sha)
            {
                continue;
            }
            let per_repo_override = branch_watch.per_repo.get(&pr.repo).map(Vec::as_slice);
            let detected = state
                .protected_branches
                .get(&pr.repo)
                .map(|c| c.branches.as_slice())
                .unwrap_or(&[]);
            let watched = resolve_watched_branches(
                per_repo_override,
                detected,
                &branch_watch.global,
                &pr.default_branch,
            );
            if is_unwatched_branch(&pr, &watched) {
                continue;
            }
            out.push(pr);
        }
    }
    out.sort_by_key(|pr| {
        (
            !pr.asked_for_you,
            u64::from(pr.additions) + u64::from(pr.deletions),
            pr.id.clone(),
        )
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Author, CheckState};

    fn pr(id: &str, repo: &str, sha: &str) -> PullRequest {
        pr_with(id, repo, sha, false)
    }

    fn pr_with(id: &str, repo: &str, sha: &str, asked_for_you: bool) -> PullRequest {
        pr_sized(id, repo, sha, asked_for_you, 0, 0)
    }

    fn pr_sized(
        id: &str,
        repo: &str,
        sha: &str,
        asked_for_you: bool,
        additions: u32,
        deletions: u32,
    ) -> PullRequest {
        PullRequest {
            id: id.into(),
            number: 1,
            title: "t".into(),
            body: String::new(),
            url: String::new(),
            repo: repo.into(),
            author: Author {
                login: "a".into(),
                avatar_url: String::new(),
            },
            created_at: "2026-08-17T00:00:00Z".into(),
            additions,
            deletions,
            changed_files: 0,
            labels: vec![],
            head_sha: sha.into(),
            checks: CheckState::None,
            is_draft: false,
            // Empty on purpose: `is_unwatched_branch` treats an unknown
            // base branch as "can't judge, don't filter", so leaving
            // these blank keeps every existing test in this module
            // unaffected by the branch filter. Tests exercising the
            // filter itself set these explicitly -- see `pr_targeting`.
            base_ref_name: String::new(),
            default_branch: String::new(),
            asked_for_you,
        }
    }

    /// Like `pr_sized`, but with an explicit base/default branch pair --
    /// used only by the branch-filter tests below.
    fn pr_targeting(id: &str, repo: &str, base: &str, default_branch: &str) -> PullRequest {
        PullRequest {
            base_ref_name: base.into(),
            default_branch: default_branch.into(),
            ..pr(id, repo, "s")
        }
    }

    #[test]
    fn deduplicates_a_pr_returned_by_two_rules() {
        let a = vec![pr("PR_1", "o/r", "sha")];
        let b = vec![pr("PR_1", "o/r", "sha"), pr("PR_2", "o/r", "sha")];
        let out = assemble(
            vec![a, b],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "PR_1");
        assert_eq!(out[1].id, "PR_2");
    }

    #[test]
    fn hides_a_muted_pr_while_its_head_sha_is_unchanged() {
        let mut state = AppState::default();
        state.muted.insert("PR_1".into(), "sha_a".into());
        let out = assemble(
            vec![vec![pr("PR_1", "o/r", "sha_a")]],
            &state,
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_muted_pr_reappears_after_a_new_commit() {
        // This is the entire meaning of "mute until it changes".
        let mut state = AppState::default();
        state.muted.insert("PR_1".into(), "sha_a".into());
        let out = assemble(
            vec![vec![pr("PR_1", "o/r", "sha_b")]],
            &state,
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn drops_prs_from_muted_repos() {
        let repos = RepoConfig {
            muted: ["acme/noisy".to_string()].into_iter().collect(),
        };
        let out = assemble(
            vec![vec![
                pr("PR_1", "acme/noisy", "s"),
                pr("PR_2", "o/keep", "s"),
            ]],
            &AppState::default(),
            &repos,
            &BranchWatch::default(),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].repo, "o/keep");
    }

    #[test]
    fn empty_input_yields_an_empty_inbox() {
        assert!(assemble(
            vec![],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default()
        )
        .is_empty());
    }

    #[test]
    fn a_pr_in_both_batches_keeps_the_asked_for_you_flag_when_that_batch_is_first() {
        // The whole trick this depends on: dedupe is first-occurrence-wins,
        // so the poller only has to pass the asked-for-you batch first and
        // pre-flag its own PRs -- no special-casing needed here.
        let asked = vec![pr_with("PR_1", "o/r", "sha", true)];
        let anyone = vec![pr_with("PR_1", "o/r", "sha", false)];
        let out = assemble(
            vec![asked, anyone],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].asked_for_you);
    }

    #[test]
    fn a_pr_only_in_the_anyone_batch_is_not_flagged() {
        let anyone = vec![pr_with("PR_1", "o/r", "sha", false)];
        let out = assemble(
            vec![anyone],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert!(!out[0].asked_for_you);
    }

    #[test]
    fn asked_for_you_prs_sort_before_others_but_the_sort_is_stable_within_each_group() {
        let results = vec![vec![
            pr_with("PR_1", "o/a", "s", false),
            pr_with("PR_2", "o/a", "s", true),
            pr_with("PR_3", "o/b", "s", false),
            pr_with("PR_4", "o/b", "s", true),
            pr_with("PR_5", "o/a", "s", false),
        ]];
        let out = assemble(
            results,
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        // Both asked-for-you PRs come first, in their original relative
        // order, followed by the unflagged ones in their original
        // relative order.
        assert_eq!(
            out.iter().map(|pr| pr.id.as_str()).collect::<Vec<_>>(),
            vec!["PR_2", "PR_4", "PR_1", "PR_3", "PR_5"]
        );
    }

    #[test]
    fn a_flagged_pr_lands_at_the_top_of_its_own_repo_group_once_grouped() {
        // Mirrors what the frontend does: group by repo, preserving the
        // order `assemble` already produced. This is the property the
        // whole stable-sort design depends on -- confirmed here rather
        // than only asserted in a comment.
        let results = vec![vec![
            pr_with("PR_1", "o/a", "s", false),
            pr_with("PR_2", "o/a", "s", false),
            pr_with("PR_3", "o/a", "s", true),
        ]];
        let out = assemble(
            results,
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );

        let mut groups: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        for pr in &out {
            groups.entry(&pr.repo).or_default().push(&pr.id);
        }
        assert_eq!(groups["o/a"], vec!["PR_3", "PR_1", "PR_2"]);
    }

    #[test]
    fn a_flagged_pr_still_sorts_before_an_unflagged_one_even_when_much_bigger() {
        // The size sort must never leak across the asked_for_you tier
        // boundary: a huge flagged PR still beats a tiny unflagged one.
        let big_flagged = pr_sized("PR_1", "o/a", "s", true, 800, 400);
        let tiny_unflagged = pr_sized("PR_2", "o/a", "s", false, 5, 3);
        let out = assemble(
            vec![vec![tiny_unflagged, big_flagged]],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(
            out.iter().map(|pr| pr.id.as_str()).collect::<Vec<_>>(),
            vec!["PR_1", "PR_2"]
        );
    }

    #[test]
    fn within_a_tier_smaller_diffs_sort_first() {
        let results = vec![vec![
            pr_sized("PR_big", "o/a", "s", false, 600, 296),
            pr_sized("PR_small", "o/a", "s", false, 65, 53),
            pr_sized("PR_mid", "o/a", "s", false, 200, 130),
        ]];
        let out = assemble(
            results,
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(
            out.iter().map(|pr| pr.id.as_str()).collect::<Vec<_>>(),
            vec!["PR_small", "PR_mid", "PR_big"]
        );
    }

    #[test]
    fn equal_size_prs_tie_break_deterministically_on_id_regardless_of_input_order() {
        // The whole point of the tie-break: GitHub's own result order is
        // not guaranteed stable across polls, so two equal-size PRs must
        // land in the same relative order regardless of which order the
        // API happened to return them in this cycle.
        let a = pr_sized("PR_b", "o/a", "s", false, 40, 10);
        let b = pr_sized("PR_a", "o/a", "s", false, 40, 10);

        let forward = assemble(
            vec![vec![a.clone(), b.clone()]],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        let reversed = assemble(
            vec![vec![b, a]],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );

        let ids = |out: &[PullRequest]| out.iter().map(|pr| pr.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&forward), vec!["PR_a", "PR_b"]);
        assert_eq!(ids(&reversed), vec!["PR_a", "PR_b"]);
    }

    // -- resolve_watched_branches -------------------------------------

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tier_1_a_per_repo_override_wins_outright() {
        let over = strs(&["release"]);
        let detected = strs(&["main", "dev"]);
        let global = strs(&["main"]);
        assert_eq!(
            resolve_watched_branches(Some(&over), &detected, &global, "main"),
            over
        );
    }

    #[test]
    fn tier_1_an_empty_override_does_not_win_it_falls_through() {
        // `Some(&[])` -- the override key exists but was cleared -- must
        // not be treated as "watch nothing"; it must fall to the next
        // tier exactly like `None` would.
        let over: Vec<String> = vec![];
        let detected = strs(&["main", "dev"]);
        assert_eq!(
            resolve_watched_branches(Some(&over), &detected, &strs(&["main"]), "main"),
            detected
        );
    }

    #[test]
    fn tier_2_detected_protected_branches_win_over_the_global_list() {
        // The atlas repo case this whole feature exists for: three
        // protected branches (dev, main, prod), not just the default.
        let detected = strs(&["dev", "main", "prod"]);
        let global = strs(&["main", "master", "develop"]);
        assert_eq!(
            resolve_watched_branches(None, &detected, &global, "main"),
            detected
        );
    }

    #[test]
    fn tier_3_an_empty_detected_list_falls_through_to_the_global_list() {
        // The `rusty-ferris-club/rust-starter` case: `branches?protected=true`
        // came back empty, which is ambiguous between "nothing protected"
        // and "no permission to see it" -- verified live, there is no way
        // to tell the two apart. Either way, this tier must fall through
        // rather than trust the empty result as "watch nothing".
        let detected: Vec<String> = vec![];
        let global = strs(&["main", "master", "develop"]);
        assert_eq!(
            resolve_watched_branches(None, &detected, &global, "main"),
            global
        );
    }

    #[test]
    fn tier_4_an_emptied_global_list_falls_through_to_the_default_branch() {
        let detected: Vec<String> = vec![];
        let global: Vec<String> = vec![];
        assert_eq!(
            resolve_watched_branches(None, &detected, &global, "trunk"),
            vec!["trunk".to_string()]
        );
    }

    #[test]
    fn every_tier_empty_yields_an_empty_list_not_a_hide_all_signal() {
        // The critical safety property: this must never be misread by a
        // caller as "watch zero branches" (which would hide every PR in
        // the repo). It must be read as "nothing resolved -- don't
        // filter". `is_unwatched_branch` (and, at the poller level,
        // `assemble`) is what actually enforces that reading; this test
        // just pins the raw signal it acts on.
        assert!(resolve_watched_branches(None, &[], &[], "").is_empty());
    }

    #[test]
    fn a_present_but_unused_override_key_is_distinguishable_from_no_override() {
        // Sanity check on the `Option` shape itself: `None` (no entry in
        // the per-repo map at all) and `Some(&[])` (an entry that exists
        // but was cleared) must both fall through identically -- neither
        // is "watch nothing".
        let global = strs(&["main"]);
        assert_eq!(
            resolve_watched_branches(None, &[], &global, ""),
            resolve_watched_branches(Some(&[]), &[], &global, "")
        );
    }

    // -- branch_watch_source --------------------------------------------

    #[test]
    fn branch_watch_source_names_every_tier() {
        let over = strs(&["release"]);
        let detected = strs(&["main", "dev"]);
        let global = strs(&["main"]);
        assert_eq!(
            branch_watch_source(Some(&over), &detected, &global, "main"),
            "override"
        );
        assert_eq!(
            branch_watch_source(None, &detected, &global, "main"),
            "detected"
        );
        assert_eq!(branch_watch_source(None, &[], &global, "main"), "global");
        assert_eq!(branch_watch_source(None, &[], &[], "main"), "default");
        assert_eq!(branch_watch_source(None, &[], &[], ""), "unfiltered");
    }

    // -- assemble: branch filtering --------------------------------------

    #[test]
    fn a_pr_targeting_the_default_branch_survives() {
        let out = assemble(
            vec![vec![pr_targeting("PR_1", "o/r", "main", "main")]],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_pr_targeting_a_feature_branch_is_dropped() {
        // The exact live bug report: a PR from one feature branch into
        // another, neither protected nor default, is noise.
        let out = assemble(
            vec![vec![pr_targeting(
                "PR_1165",
                "acme-labs/atlas",
                "mira/retry-reward-reports",
                "main",
            )]],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn a_pr_targeting_a_detected_protected_branch_that_is_not_the_default_survives() {
        // The atlas repo has three protected branches (dev, main, prod).
        // "default branch only" would be too narrow and wrongly hide a PR
        // targeting `dev`.
        let mut state = AppState::default();
        state.protected_branches.insert(
            "acme-labs/atlas".into(),
            crate::store::ProtectedBranchesCache {
                branches: strs(&["dev", "main", "prod"]),
                checked_at: 0,
            },
        );
        let out = assemble(
            vec![vec![pr_targeting("PR_1", "acme-labs/atlas", "dev", "main")]],
            &state,
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(
            out.len(),
            1,
            "a PR targeting a protected non-default branch must survive"
        );
    }

    #[test]
    fn a_per_repo_override_replaces_detection_for_that_repo() {
        let mut state = AppState::default();
        state.protected_branches.insert(
            "o/r".into(),
            crate::store::ProtectedBranchesCache {
                branches: strs(&["main"]),
                checked_at: 0,
            },
        );
        let branch_watch = BranchWatch {
            global: strs(&["main", "master", "develop"]),
            per_repo: [("o/r".to_string(), strs(&["release"]))]
                .into_iter()
                .collect(),
        };
        // Detection says only `main` is protected, but the user has
        // overridden this repo to `release` -- the override must win, so
        // a PR targeting `main` (not in the override) is dropped and one
        // targeting `release` survives.
        let out = assemble(
            vec![vec![
                pr_targeting("PR_main", "o/r", "main", "main"),
                pr_targeting("PR_release", "o/r", "release", "main"),
            ]],
            &state,
            &RepoConfig::default(),
            &branch_watch,
        );
        assert_eq!(
            out.iter().map(|pr| pr.id.as_str()).collect::<Vec<_>>(),
            vec!["PR_release"]
        );
    }

    #[test]
    fn an_unresolvable_watch_set_does_not_hide_the_pr() {
        // Pins the exact failure mode this feature must never reintroduce:
        // a bug in resolution (a fetch failure, an emptied global list,
        // an unknown default branch) must never silently empty the whole
        // inbox. With every tier empty, the PR must survive.
        let branch_watch = BranchWatch {
            global: vec![],
            per_repo: Default::default(),
        };
        let out = assemble(
            vec![vec![pr_targeting("PR_1", "o/r", "some-feature-branch", "")]],
            &AppState::default(),
            &RepoConfig::default(),
            &branch_watch,
        );
        assert_eq!(
            out.len(),
            1,
            "an empty resolved watch-set must mean 'don't filter', not 'hide everything'"
        );
    }

    #[test]
    fn an_unknown_base_branch_on_the_pr_itself_is_never_filtered() {
        // If GitHub's response shape ever omits `baseRefName`, the PR
        // must not be silently dropped -- `base_ref_name` defaults to ""
        // in that case (see models::node_to_pr), and that must read as
        // "can't judge this" rather than "this PR fails the filter".
        let out = assemble(
            vec![vec![pr_targeting("PR_1", "o/r", "", "main")]],
            &AppState::default(),
            &RepoConfig::default(),
            &BranchWatch::default(),
        );
        assert_eq!(out.len(), 1);
    }
}

/// One repo's resolved "branches we care about", plus where that answer
/// came from.
///
/// This exists because the branch filter *hides* PRs: a pull request
/// from one feature branch into another gates nothing and is dropped
/// from the inbox entirely. Silent hiding is exactly the failure mode
/// this project has been bitten by before, so the Settings screen owes
/// the user one place that explains, per repo, which branches count and
/// which tier decided that.
#[derive(Debug, Clone, PartialEq)]
pub struct BranchWatchInfo {
    pub repo: String,
    pub branches: Vec<String>,
    /// `"override"`, `"detected"`, `"global"`, `"default"`, or
    /// `"unfiltered"` — nothing resolved, so this repo is not filtered
    /// by branch at all.
    pub source: &'static str,
}

/// Every repo hi5 has seen a PR from, plus any repo carrying only a
/// per-repo override, each resolved exactly the way `assemble` resolves
/// it live.
///
/// Computed from the given settings and state rather than cached, so an
/// edit in Settings is reflected the moment it is saved instead of
/// waiting for the next poll cycle.
pub fn branch_watch_status(
    settings: &crate::store::Settings,
    state: &crate::store::AppState,
) -> Vec<BranchWatchInfo> {
    let mut repos: std::collections::BTreeSet<String> =
        state.repo_defaults.keys().cloned().collect();
    repos.extend(state.protected_branches.keys().cloned());
    repos.extend(settings.branch_watch.per_repo.keys().cloned());

    repos
        .into_iter()
        .map(|repo| {
            let per_repo_override = settings.branch_watch.per_repo.get(&repo).map(Vec::as_slice);
            let detected = state
                .protected_branches
                .get(&repo)
                .map(|c| c.branches.as_slice())
                .unwrap_or(&[]);
            let default_branch = state.repo_defaults.get(&repo).map_or("", String::as_str);
            BranchWatchInfo {
                branches: resolve_watched_branches(
                    per_repo_override,
                    detected,
                    &settings.branch_watch.global,
                    default_branch,
                ),
                source: branch_watch_source(
                    per_repo_override,
                    detected,
                    &settings.branch_watch.global,
                    default_branch,
                ),
                repo,
            }
        })
        .collect()
}
