use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// One repo's cached `branches?protected=true` result plus when it was
/// fetched -- see `poller::should_refresh_protection`. Protection status
/// changes rarely, so this is refetched at most once a day per repo
/// rather than once every poll cycle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProtectedBranchesCache {
    /// Empty covers two cases the API makes indistinguishable: the repo
    /// genuinely has no protected branches, or the token lacks
    /// permission to see them. Either way `resolve_watched_branches`
    /// falls through to the global list -- this cache never disambiguates
    /// that, it only remembers what GitHub last said.
    pub branches: Vec<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppState {
    /// pr_node_id -> head_sha at the moment it was muted.
    pub muted: HashMap<String, String>,
    /// "{pr_node_id}:{head_sha}" pairs already notified about.
    pub notified: HashSet<String>,
    /// pr_node_id -> unix seconds of last sighting, for pruning.
    pub last_seen: HashMap<String, i64>,
    /// repo (`nameWithOwner`) -> cached protected-branches lookup.
    pub protected_branches: HashMap<String, ProtectedBranchesCache>,
    /// repo (`nameWithOwner`) -> `repository.defaultBranchRef.name` last
    /// seen for it. Kept alongside `protected_branches` so the Settings
    /// screen can explain *why* a repo resolved to the branches it did
    /// (including the last-resort "default branch" tier) without a live
    /// network call -- see `commands::get_branch_watch_status`.
    pub repo_defaults: HashMap<String, String>,
}

pub const PRUNE_AFTER_SECS: i64 = 7 * 24 * 60 * 60;

pub fn notified_key(pr_id: &str, head_sha: &str) -> String {
    format!("{pr_id}:{head_sha}")
}

impl AppState {
    pub fn touch(&mut self, pr_id: &str, now: i64) {
        self.last_seen.insert(pr_id.to_string(), now);
    }

    /// Drop bookkeeping for PRs not seen in a week so the files
    /// don't grow without bound.
    pub fn prune(&mut self, now: i64) {
        let stale: Vec<String> = self
            .last_seen
            .iter()
            .filter(|(_, &seen)| now - seen > PRUNE_AFTER_SECS)
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            self.last_seen.remove(&id);
            self.muted.remove(&id);
            self.notified.retain(|k| !k.starts_with(&format!("{id}:")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_drops_entries_older_than_a_week() {
        let mut s = AppState::default();
        s.muted.insert("PR_old".into(), "sha1".into());
        s.notified.insert(notified_key("PR_old", "sha1"));
        s.last_seen.insert("PR_old".into(), 0);

        s.muted.insert("PR_new".into(), "sha2".into());
        s.notified.insert(notified_key("PR_new", "sha2"));
        s.last_seen.insert("PR_new".into(), PRUNE_AFTER_SECS);

        s.prune(PRUNE_AFTER_SECS + 1);

        assert!(!s.muted.contains_key("PR_old"));
        assert!(!s.notified.contains(&notified_key("PR_old", "sha1")));
        assert!(s.muted.contains_key("PR_new"));
        assert!(s.notified.contains(&notified_key("PR_new", "sha2")));
    }

    #[test]
    fn a_state_file_from_before_branch_watch_existed_still_loads() {
        // Migration: an older state.json has neither key at all.
        let json = r#"{"muted":{},"notified":[],"lastSeen":{}}"#;
        let s: AppState = serde_json::from_str(json).unwrap();
        assert!(s.protected_branches.is_empty());
        assert!(s.repo_defaults.is_empty());
    }

    #[test]
    fn pruning_a_stale_pr_does_not_touch_the_repo_level_branch_caches() {
        // protected_branches/repo_defaults are keyed by repo, not PR id --
        // `prune` operates on last_seen's PR-id keys and must leave these
        // alone regardless of how stale an unrelated PR is.
        let mut s = AppState::default();
        s.protected_branches.insert(
            "o/r".into(),
            ProtectedBranchesCache {
                branches: vec!["main".into()],
                checked_at: 0,
            },
        );
        s.repo_defaults.insert("o/r".into(), "main".into());
        s.last_seen.insert("PR_old".into(), 0);

        s.prune(PRUNE_AFTER_SECS + 1);

        assert!(s.protected_branches.contains_key("o/r"));
        assert_eq!(s.repo_defaults.get("o/r").map(String::as_str), Some("main"));
    }
}
