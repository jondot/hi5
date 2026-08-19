use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckState {
    Success,
    Failure,
    Pending,
    /// The repo has no CI configured; render nothing rather than a
    /// misleading red or green indicator.
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub repo: String,
    pub author: Author,
    pub created_at: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub labels: Vec<Label>,
    pub head_sha: String,
    pub checks: CheckState,
    pub is_draft: bool,
    /// The branch this PR wants to merge into (`baseRefName`). Used by
    /// `inbox::resolve_watched_branches` to decide whether the PR gates
    /// anything the viewer cares about -- a PR from one feature branch
    /// back to another (neither protected, neither the default) is noise
    /// in a shared review queue.
    pub base_ref_name: String,
    /// `repository.defaultBranchRef.name` at fetch time -- the last-resort
    /// tier of `resolve_watched_branches`, and what the frontend compares
    /// `base_ref_name` against to decide whether a row's target branch is
    /// unusual enough to show. Empty when GitHub reports no default branch
    /// (an empty repo), which is rare enough to just fall through.
    pub default_branch: String,
    /// Not a GitHub field -- derived from which query returned this PR.
    /// `node_to_pr` always sets this `false`; the poller flips it to
    /// `true` for every PR in the "asked for you" batch before handing
    /// results to `inbox::assemble` (see poller.rs `cycle`).
    pub asked_for_you: bool,
}

/// Parses a GraphQL search response into PRs, alongside the raw node
/// count seen before per-node parsing. The count lets a caller tell
/// "GitHub returned nothing" apart from "GitHub returned nodes but every
/// one of them failed to parse" -- the latter means the response shape
/// changed and silently returning an empty `Vec` would hide that.
pub fn parse_search_response(raw: &str) -> crate::error::Result<(Vec<PullRequest>, usize)> {
    use crate::error::AppError;
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| AppError::GitHub(e.to_string()))?;

    // Surface top-level GraphQL errors, but still return whatever
    // nodes resolved -- a partial inbox beats an empty one.
    let nodes = v
        .pointer("/data/search/nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| {
            let msg = v
                .pointer("/errors/0/message")
                .and_then(|m| m.as_str())
                .unwrap_or("malformed search response");
            AppError::GitHub(msg.to_string())
        })?;

    let raw_count = nodes.len();
    let prs = nodes.iter().filter_map(node_to_pr).collect();
    Ok((prs, raw_count))
}

fn node_to_pr(n: &serde_json::Value) -> Option<PullRequest> {
    let commit = n.pointer("/commits/nodes/0/commit")?;
    let checks = match commit
        .pointer("/statusCheckRollup/state")
        .and_then(|s| s.as_str())
    {
        Some("SUCCESS") => CheckState::Success,
        Some("FAILURE") | Some("ERROR") => CheckState::Failure,
        Some("PENDING") | Some("EXPECTED") => CheckState::Pending,
        _ => CheckState::None,
    };

    Some(PullRequest {
        id: n.get("id")?.as_str()?.to_string(),
        number: n.get("number")?.as_u64()?,
        title: n.get("title")?.as_str()?.to_string(),
        body: n
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string(),
        url: n.get("url")?.as_str()?.to_string(),
        repo: n
            .pointer("/repository/nameWithOwner")?
            .as_str()?
            .to_string(),
        author: Author {
            login: n
                .pointer("/author/login")
                .and_then(|l| l.as_str())
                .unwrap_or("ghost")
                .to_string(),
            avatar_url: n
                .pointer("/author/avatarUrl")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string(),
        },
        created_at: n.get("createdAt")?.as_str()?.to_string(),
        additions: n.get("additions").and_then(|a| a.as_u64()).unwrap_or(0) as u32,
        deletions: n.get("deletions").and_then(|a| a.as_u64()).unwrap_or(0) as u32,
        changed_files: n.get("changedFiles").and_then(|a| a.as_u64()).unwrap_or(0) as u32,
        labels: n
            .pointer("/labels/nodes")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| {
                        Some(Label {
                            name: l.get("name")?.as_str()?.to_string(),
                            color: l.get("color")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        head_sha: commit.get("oid")?.as_str()?.to_string(),
        checks,
        is_draft: n.get("isDraft").and_then(|d| d.as_bool()).unwrap_or(false),
        // Neither field is treated as required (`?`): an unknown base
        // branch must never drop an otherwise-valid PR out of the inbox --
        // `inbox::assemble` treats an empty `base_ref_name` as "can't
        // judge, so don't filter" rather than failing the whole parse.
        base_ref_name: n
            .get("baseRefName")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string(),
        default_branch: n
            .pointer("/repository/defaultBranchRef/name")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string(),
        asked_for_you: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/search_response.json");

    #[test]
    fn parses_all_prs_from_the_fixture() {
        let (prs, raw_count) = parse_search_response(FIXTURE).unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(raw_count, 2);
    }

    #[test]
    fn maps_every_field_of_the_first_pr() {
        let (prs, _) = parse_search_response(FIXTURE).unwrap();
        let pr = &prs[0];
        assert_eq!(pr.id, "PR_kwDOabc123");
        assert_eq!(pr.number, 1420);
        assert_eq!(pr.repo, "loco-rs/loco");
        assert_eq!(pr.author.login, "alice");
        assert_eq!(pr.head_sha, "a3f9c21");
        assert_eq!(pr.checks, CheckState::Success);
        assert_eq!(pr.additions, 12);
        assert_eq!(pr.labels.len(), 1);
        assert_eq!(pr.base_ref_name, "main");
        assert_eq!(pr.default_branch, "main");
    }

    #[test]
    fn a_missing_base_branch_or_default_branch_ref_defaults_to_empty_not_a_dropped_node() {
        // Neither field is required with `?` -- an unknown base branch
        // must never drop an otherwise-valid PR from parsing. The second
        // fixture node has no `baseRefName` and no `defaultBranchRef` at
        // all, mirroring a response shape hi5 hasn't seen yet.
        let (prs, _) = parse_search_response(FIXTURE).unwrap();
        let pr = &prs[1];
        assert_eq!(pr.base_ref_name, "");
        assert_eq!(pr.default_branch, "");
    }

    #[test]
    fn a_freshly_parsed_pr_is_never_pre_flagged_asked_for_you() {
        // That flag is derived by the poller from which query returned
        // the PR, not by GitHub -- parsing must always start it false.
        let (prs, _) = parse_search_response(FIXTURE).unwrap();
        assert!(prs.iter().all(|pr| !pr.asked_for_you));
    }

    #[test]
    fn absent_status_rollup_maps_to_none_not_failure() {
        // A repo with no CI must not render as a red X.
        let (prs, _) = parse_search_response(FIXTURE).unwrap();
        let pr = &prs[1];
        assert_eq!(pr.checks, CheckState::None);
        assert_eq!(pr.body, "");
        assert!(pr.labels.is_empty());
    }

    #[test]
    fn graphql_errors_surface_as_an_error() {
        let raw = r#"{"errors":[{"message":"Bad credentials"}]}"#;
        let err = parse_search_response(raw).unwrap_err();
        assert!(err.to_string().contains("Bad credentials"));
    }

    #[test]
    fn a_malformed_node_is_skipped_rather_than_failing_the_batch() {
        let raw = r#"{"data":{"search":{"nodes":[{"id":"x"}]}}}"#;
        let (prs, raw_count) = parse_search_response(raw).unwrap();
        assert_eq!(prs.len(), 0);
        assert_eq!(raw_count, 1);
    }

    #[test]
    fn all_nodes_failing_to_parse_reports_the_raw_count_for_diagnosis() {
        // If GitHub renames a field, every node can fail to parse at once.
        // The raw count is what lets a caller tell that apart from a
        // genuinely empty inbox and surface it instead of failing silently.
        let raw = r#"{"data":{"search":{"nodes":[{"id":"a"},{"id":"b"},{"id":"c"}]}}}"#;
        let (prs, raw_count) = parse_search_response(raw).unwrap();
        assert!(prs.is_empty());
        assert_eq!(raw_count, 3);
    }
}
