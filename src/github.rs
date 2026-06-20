//! Thin async GitHub REST client plus the data structures `repoforge` reasons about.
//!
//! Only the handful of endpoints the auditor needs are implemented. Every call sets the
//! `User-Agent` header GitHub requires and, when a token is present, authenticates so the
//! 5 000 req/hour limit applies instead of the 60 req/hour anonymous one.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use reqwest::{header, Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;

const API: &str = "https://api.github.com";
const UA: &str = concat!("repoforge/", env!("CARGO_PKG_VERSION"));

/// Repository metadata as returned by `GET /repos/{owner}/{repo}`.
#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub name: String,
    pub full_name: String,
    pub owner: Owner,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub license: Option<License>,
    #[serde(default = "default_branch")]
    pub default_branch: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub stargazers_count: u32,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub pushed_at: Option<String>,
}

fn default_branch() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Owner {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct License {
    #[serde(default)]
    pub spdx_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Everything fetched for a single repository, ready to be scored.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub repo: Repo,
    /// Every file path in the default branch (lower-cased for matching is done by callers).
    pub paths: Vec<String>,
    /// Decoded README contents, if the repo has one.
    pub readme: Option<String>,
    /// True when the git tree was truncated by the API (very large repos).
    pub tree_truncated: bool,
}

#[derive(Deserialize)]
struct TreeResp {
    #[serde(default)]
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct ContentResp {
    #[serde(default)]
    content: String,
    #[serde(default)]
    encoding: String,
}

pub struct GitHub {
    client: Client,
}

impl GitHub {
    pub fn new(token: Option<String>) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            header::HeaderValue::from_static("2022-11-28"),
        );
        if let Some(tok) = token {
            let mut val = header::HeaderValue::from_str(&format!("Bearer {tok}"))
                .context("invalid token characters")?;
            val.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, val);
        }
        let client = Client::builder()
            .user_agent(UA)
            .default_headers(headers)
            .build()
            .context("building HTTP client")?;
        Ok(Self { client })
    }

    /// List every non-fork, non-archived public repo owned by `user` (filters configurable).
    pub async fn list_user_repos(
        &self,
        user: &str,
        include_forks: bool,
        include_archived: bool,
    ) -> Result<Vec<Repo>> {
        let mut out = Vec::new();
        let mut page = 1u32;
        loop {
            let url = format!("{API}/users/{user}/repos?per_page=100&type=owner&sort=pushed&page={page}");
            let resp = self.get_retry(&url).await?;
            let resp = check(resp).await?;
            let batch: Vec<Repo> = resp.json().await.context("decoding repo list")?;
            let n = batch.len();
            out.extend(batch);
            if n < 100 {
                break;
            }
            page += 1;
            if page > 50 {
                break; // 5 000-repo hard stop, well beyond any real account
            }
        }
        out.retain(|r| (include_forks || !r.fork) && (include_archived || !r.archived));
        Ok(out)
    }

    pub async fn get_repo(&self, owner: &str, name: &str) -> Result<Repo> {
        let url = format!("{API}/repos/{owner}/{name}");
        let resp = check(self.get_retry(&url).await?).await?;
        resp.json().await.context("decoding repo")
    }

    /// GET with retry + backoff. Returns the raw (post-retry) response so callers can still
    /// inspect 404/409 statuses themselves.
    async fn get_retry(&self, url: &str) -> Result<reqwest::Response> {
        self.send_retry(|| self.client.get(url)).await
    }

    /// Send a request with retry + exponential backoff on transient failures: 429, 5xx, and the
    /// 403 + `Retry-After` shape GitHub uses for *secondary* rate limits on writes. `build` is
    /// called afresh each attempt because a `RequestBuilder` is consumed by `send`. Honours
    /// `Retry-After` when present, otherwise backs off 1/2/4 seconds.
    async fn send_retry<F>(&self, build: F) -> Result<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        const MAX: u32 = 5;
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match build().send().await {
                Ok(resp) => {
                    let s = resp.status();
                    let has_retry_after = resp.headers().contains_key("retry-after");
                    let secondary = s == StatusCode::FORBIDDEN && has_retry_after;
                    let retryable = s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error() || secondary;
                    if retryable && attempt < MAX {
                        let secs = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(1u64 << (attempt - 1)); // 1, 2, 4, 8 seconds
                        tokio::time::sleep(Duration::from_secs(secs.min(60))).await;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(_) if attempt < MAX => {
                    tokio::time::sleep(Duration::from_millis(300 * attempt as u64)).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Build a full [`Snapshot`]: metadata + recursive file tree + README contents.
    pub async fn snapshot(&self, repo: Repo) -> Result<Snapshot> {
        let (owner, name) = (repo.owner.login.clone(), repo.name.clone());
        let (tree, readme) = futures::join!(
            self.tree(&owner, &name, &repo.default_branch),
            self.readme(&owner, &name),
        );
        let (paths, truncated) = tree.unwrap_or_else(|_| (Vec::new(), false));
        Ok(Snapshot {
            repo,
            paths,
            readme: readme.unwrap_or(None),
            tree_truncated: truncated,
        })
    }

    async fn tree(&self, owner: &str, name: &str, branch: &str) -> Result<(Vec<String>, bool)> {
        let url = format!("{API}/repos/{owner}/{name}/git/trees/{branch}?recursive=1");
        let resp = self.get_retry(&url).await?;
        if resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::CONFLICT {
            return Ok((Vec::new(), false)); // empty repo
        }
        let resp = check(resp).await?;
        let body: TreeResp = resp.json().await.context("decoding git tree")?;
        let paths = body
            .tree
            .into_iter()
            .filter(|e| e.kind == "blob" || e.kind == "tree")
            .map(|e| e.path)
            .collect();
        Ok((paths, body.truncated))
    }

    async fn readme(&self, owner: &str, name: &str) -> Result<Option<String>> {
        let url = format!("{API}/repos/{owner}/{name}/readme");
        let resp = self.get_retry(&url).await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = check(resp).await?;
        let body: ContentResp = resp.json().await.context("decoding readme")?;
        if body.encoding != "base64" {
            return Ok(Some(body.content));
        }
        let cleaned: String = body.content.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(cleaned.as_bytes())
            .context("base64-decoding readme")?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }

    // ---- mutation (used by `repoforge fix --apply`) ----

    /// PATCH repo metadata (description / homepage). `body` is the raw JSON patch.
    pub async fn patch_repo(&self, owner: &str, name: &str, body: serde_json::Value) -> Result<()> {
        let url = format!("{API}/repos/{owner}/{name}");
        let resp = self.send_retry(|| self.client.patch(&url).json(&body)).await?;
        check(resp).await?;
        Ok(())
    }

    pub async fn replace_topics(&self, owner: &str, name: &str, topics: &[String]) -> Result<()> {
        let url = format!("{API}/repos/{owner}/{name}/topics");
        let body = serde_json::json!({ "names": topics });
        let resp = self.send_retry(|| self.client.put(&url).json(&body)).await?;
        check(resp).await?;
        Ok(())
    }

    /// Create a file at `path` (only used when the file is known to be absent, so no sha needed).
    /// When `branch` is `Some`, the file is committed to that branch instead of the default.
    pub async fn put_file(
        &self,
        owner: &str,
        name: &str,
        path: &str,
        message: &str,
        contents: &str,
        branch: Option<&str>,
    ) -> Result<()> {
        let url = format!("{API}/repos/{owner}/{name}/contents/{path}");
        let encoded = base64::engine::general_purpose::STANDARD.encode(contents.as_bytes());
        let mut body = serde_json::json!({ "message": message, "content": encoded });
        if let Some(b) = branch {
            body["branch"] = serde_json::Value::String(b.to_string());
        }
        let resp = self.send_retry(|| self.client.put(&url).json(&body)).await?;
        check(resp).await?;
        Ok(())
    }

    /// Current head commit SHA of `branch`.
    pub async fn head_sha(&self, owner: &str, name: &str, branch: &str) -> Result<String> {
        let url = format!("{API}/repos/{owner}/{name}/git/ref/heads/{branch}");
        let resp = check(self.get_retry(&url).await?).await?;
        let r: RefResp = resp.json().await.context("decoding ref")?;
        Ok(r.object.sha)
    }

    /// Create `new_branch` pointing at `from_sha`. Tolerates "already exists" (422).
    pub async fn create_branch(
        &self,
        owner: &str,
        name: &str,
        new_branch: &str,
        from_sha: &str,
    ) -> Result<()> {
        let url = format!("{API}/repos/{owner}/{name}/git/refs");
        let body = serde_json::json!({ "ref": format!("refs/heads/{new_branch}"), "sha": from_sha });
        let resp = self.send_retry(|| self.client.post(&url).json(&body)).await?;
        if resp.status() == StatusCode::UNPROCESSABLE_ENTITY {
            return Ok(()); // branch already exists — reuse it
        }
        check(resp).await?;
        Ok(())
    }

    /// Open a pull request and return its URL. Tolerates "already exists" (422).
    pub async fn open_pr(
        &self,
        owner: &str,
        name: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<String> {
        let url = format!("{API}/repos/{owner}/{name}/pulls");
        let payload = serde_json::json!({ "title": title, "head": head, "base": base, "body": body });
        let resp = self.send_retry(|| self.client.post(&url).json(&payload)).await?;
        if resp.status() == StatusCode::UNPROCESSABLE_ENTITY {
            return Ok(format!("https://github.com/{owner}/{name}/pulls (already open)"));
        }
        let resp = check(resp).await?;
        let pr: PrResp = resp.json().await.context("decoding pull request")?;
        Ok(pr.html_url)
    }
}

#[derive(Deserialize)]
struct RefResp {
    object: RefObject,
}

#[derive(Deserialize)]
struct RefObject {
    sha: String,
}

#[derive(Deserialize)]
struct PrResp {
    html_url: String,
}

/// Turn a non-2xx response into a useful error carrying the status and a snippet of the body.
async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let url = resp.url().to_string();
    let body = resp.text().await.unwrap_or_default();
    let snippet: String = body.chars().take(300).collect();
    Err(anyhow!("GitHub {status} for {url}: {snippet}"))
}
