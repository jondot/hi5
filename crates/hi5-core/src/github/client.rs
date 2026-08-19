use super::{models, queries, GitHubApi, Health, PullRequest};
use crate::error::{AppError, Result};
use async_trait::async_trait;

pub struct Client {
    http: reqwest::Client,
    token: String,
}

impl Client {
    pub fn new(token: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("hi5")
                .build()
                .expect("http client"),
            token,
        }
    }

    async fn graphql(&self, doc: &str, vars: serde_json::Value) -> Result<String> {
        let res = self
            .http
            .post("https://api.github.com/graphql")
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "query": doc, "variables": vars }))
            .send()
            .await?;

        map_status(&res)?;
        Ok(res.text().await?)
    }
}

/// Translate transport-level failures into the variants the poller
/// escalates on, so 401 and 403-rate-limit are never confused with a
/// generic error.
fn map_status(res: &reqwest::Response) -> Result<()> {
    match res.status().as_u16() {
        401 => Err(AppError::Unauthorized),
        403 | 429 => {
            let reset = res
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<i64>().ok());
            match reset {
                Some(t) => Err(AppError::RateLimited(t)),
                None => Err(AppError::GitHub("forbidden".into())),
            }
        }
        s if s >= 400 => Err(AppError::GitHub(format!("http {s}"))),
        _ => Ok(()),
    }
}

#[async_trait]
impl GitHubApi for Client {
    async fn search_prs(&self, query: &str) -> Result<(Vec<PullRequest>, usize)> {
        let body = self
            .graphql(queries::SEARCH_PRS, serde_json::json!({ "q": query }))
            .await?;
        models::parse_search_response(&body)
    }

    async fn approve(&self, pr_node_id: &str) -> Result<()> {
        let body = self
            .graphql(queries::APPROVE, serde_json::json!({ "id": pr_node_id }))
            .await?;
        let v: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| AppError::GitHub(e.to_string()))?;
        if let Some(msg) = v.pointer("/errors/0/message").and_then(|m| m.as_str()) {
            return Err(AppError::GitHub(msg.to_string()));
        }
        Ok(())
    }

    async fn health(&self) -> Result<Health> {
        let res = self
            .http
            .get("https://api.github.com/user")
            .bearer_auth(&self.token)
            .send()
            .await?;
        map_status(&res)?;
        let scope_header = res
            .headers()
            .get("x-oauth-scopes")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let v: serde_json::Value = res.json().await?;
        Ok(Health {
            login: v
                .get("login")
                .and_then(|l| l.as_str())
                .unwrap_or_default()
                .into(),
            scope_header,
        })
    }

    async fn list_orgs(&self) -> Result<Vec<String>> {
        let res = self
            .http
            .get("https://api.github.com/user/orgs")
            .bearer_auth(&self.token)
            .send()
            .await?;
        map_status(&res)?;
        let v: serde_json::Value = res.json().await?;
        let orgs = v
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| o.get("login")?.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(orgs)
    }

    async fn list_protected_branches(&self, repo: &str) -> Result<Vec<String>> {
        let url = format!("https://api.github.com/repos/{repo}/branches?protected=true");
        let res = self.http.get(&url).bearer_auth(&self.token).send().await?;
        map_status(&res)?;
        let v: serde_json::Value = res.json().await?;
        let branches = v
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| b.get("name")?.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(branches)
    }
}
