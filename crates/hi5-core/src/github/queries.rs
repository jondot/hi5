pub const SEARCH_PRS: &str = r#"
query($q: String!) {
  search(query: $q, type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        id number title body url isDraft createdAt
        additions deletions changedFiles
        author { login avatarUrl }
        baseRefName
        repository { nameWithOwner defaultBranchRef { name } }
        labels(first: 10) { nodes { name color } }
        commits(last: 1) {
          nodes { commit { oid statusCheckRollup { state } } }
        }
      }
    }
  }
}"#;

pub const APPROVE: &str = r#"
mutation($id: ID!) {
  addPullRequestReview(input: { pullRequestId: $id, event: APPROVE }) {
    pullRequestReview { id state }
  }
}"#;
