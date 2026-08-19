pub mod client;
pub mod models;
pub mod queries;

// Re-export surface, not scaffolding: these are the types `PullRequest`
// is made of, and callers name them through `crate::github::…`. Only
// `PullRequest` happens to be reached that way today, so the rest read as
// unused imports.
#[allow(unused_imports)]
pub use models::{Author, CheckState, Label, PullRequest};

use crate::error::Result;
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq)]
pub struct Health {
    pub login: String,
    pub scope_header: Option<String>,
}

#[async_trait]
pub trait GitHubApi: Send + Sync {
    /// Returns the parsed PRs alongside the raw node count the response
    /// contained. A caller sees `(vec![], 0)` for "nothing matched" and
    /// `(vec![], N>0)` for "the response shape changed and nothing could
    /// be parsed" -- the two must never be indistinguishable.
    async fn search_prs(&self, query: &str) -> Result<(Vec<PullRequest>, usize)>;
    /// Approve with an empty body. Never attach text.
    async fn approve(&self, pr_node_id: &str) -> Result<()>;
    /// REST call -- GraphQL does not return the scope header.
    async fn health(&self) -> Result<Health>;
    /// Organization logins the viewer belongs to (`GET /user/orgs`), for
    /// auto-discovering `Settings::watched_orgs` candidates. The `gh`
    /// token already carries `read:org` (verified against a live `gh`
    /// install, see the design spec), so this needs no extra scope
    /// prompt. Does not include the viewer's own personal login --
    /// callers add that via `query::merge_org_scopes`.
    async fn list_orgs(&self) -> Result<Vec<String>>;
    /// `GET /repos/{owner}/{repo}/branches?protected=true` -- the
    /// "branches we care about" for a repo, tier 2 of
    /// `inbox::resolve_watched_branches`. An empty result is genuinely
    /// ambiguous (no protected branches vs. no permission to see them --
    /// verified live against `rusty-ferris-club/rust-starter`, see
    /// final-fix-report.md), which is exactly why that tier falls
    /// through to the global list rather than being trusted as "nothing
    /// is protected, so hide everything". Callers must treat a `Err`
    /// here (403/404/network/5xx) the same way: fall through, never
    /// escalate the whole poll cycle over it.
    async fn list_protected_branches(&self, repo: &str) -> Result<Vec<String>>;
}
