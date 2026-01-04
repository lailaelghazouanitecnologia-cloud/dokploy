use std::time::Duration;
use backoff::ExponentialBackoff;
use backoff::backoff::Backoff;
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::config::GitHubConfig;
use crate::error::{GitHubError, Result};
use crate::validation::{validate_github_owner, validate_github_repo, validate_git_branch};

const GITHUB_API_BASE: &str = "https://api.github.com";
const TIMEOUT_SECONDS: u64 = 30;
const MAX_RETRIES: u32 = 3;
const INITIAL_RETRY_INTERVAL_MS: u64 = 500;
const MAX_RETRY_INTERVAL_MS: u64 = 10000;
const ARCHIVE_SIZE_LIMIT_BYTES: usize = 100 * 1024 * 1024;

pub struct GitHubProvider {
    client: Client,
    token:  String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id:             u64,
    pub name:           String,
    pub full_name:      String,
    pub default_branch: String,
    pub clone_url:      String,
    pub private:        bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub name:   String,
    pub commit: CommitRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitRef {
    pub sha: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub sha:     String,
    pub message: String,
    pub author:  Option<CommitAuthor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitAuthor {
    pub name:  String,
    pub email: String,
    pub date:  String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WebhookPayload {
    pub event_type: WebhookEvent,
    pub repository: String,
    pub branch:     String,
    pub commit_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEvent {
    Push,
    PullRequest,
    Release,
    Unknown,
}

impl GitHubProvider {
    pub fn new(config: &GitHubConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECONDS))
            .user_agent("rustdeploy/0.1.0")
            .build()
            .map_err(|e| GitHubError::RequestFailed {
                operation: "create_client",
                reason: e.to_string(),
            })?;

        Ok(Self {
            client,
            token: config.token.clone(),
        })
    }

    fn create_backoff() -> ExponentialBackoff {
        ExponentialBackoff {
            initial_interval: Duration::from_millis(INITIAL_RETRY_INTERVAL_MS),
            max_interval: Duration::from_millis(MAX_RETRY_INTERVAL_MS),
            max_elapsed_time: Some(Duration::from_secs(60)),
            ..Default::default()
        }
    }

    async fn handle_response(&self, response: Response, owner: &str, repo: &str) -> Result<Response> {
        let status = response.status();

        if status == StatusCode::UNAUTHORIZED {
            return Err(GitHubError::AuthFailed.into());
        }

        if status == StatusCode::FORBIDDEN {
            if let Some(retry_after) = response.headers().get("retry-after") {
                let seconds = retry_after
                    .to_str()
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60);
                return Err(GitHubError::RateLimited { retry_after_seconds: seconds }.into());
            }

            let remaining = response.headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u32>().ok());

            if remaining == Some(0) {
                return Err(GitHubError::RateLimited { retry_after_seconds: 60 }.into());
            }
        }

        if status == StatusCode::NOT_FOUND {
            return Err(GitHubError::RepoNotFound {
                owner: owner.to_string(),
                repo: repo.to_string(),
            }.into());
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GitHubError::InvalidResponse { reason: format!("{status}: {body}") }.into());
        }

        Ok(response)
    }

    async fn request_with_retry<F, Fut>(&self, operation: &'static str, mut f: F) -> Result<Response>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Response, reqwest::Error>>,
    {
        let mut backoff = Self::create_backoff();
        let mut attempts = 0u32;

        loop {
            attempts += 1;

            let result = f().await;

            match result {
                Ok(response) => {
                    let status = response.status();

                    if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
                        if attempts >= MAX_RETRIES {
                            return Err(GitHubError::RateLimited { retry_after_seconds: 0 }.into());
                        }

                        if let Some(duration) = backoff.next_backoff() {
                            tokio::time::sleep(duration).await;
                            continue;
                        }
                    }

                    return Ok(response);
                }
                Err(e) => {
                    if attempts >= MAX_RETRIES {
                        return Err(GitHubError::RequestFailed {
                            operation,
                            reason: e.to_string(),
                        }.into());
                    }

                    if e.is_timeout() || e.is_connect() {
                        if let Some(duration) = backoff.next_backoff() {
                            tokio::time::sleep(duration).await;
                            continue;
                        }
                    }

                    return Err(GitHubError::RequestFailed {
                        operation,
                        reason: e.to_string(),
                    }.into());
                }
            }
        }
    }

    pub async fn get_repository(&self, owner: &str, repo: &str) -> Result<Repository> {
        validate_github_owner(owner)?;
        validate_github_repo(repo)?;

        let encoded_owner = urlencoding::encode(owner);
        let encoded_repo = urlencoding::encode(repo);
        let url = format!("{GITHUB_API_BASE}/repos/{encoded_owner}/{encoded_repo}");

        let response = self.request_with_retry("get_repository", || {
            self.client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
        }).await?;

        let response = self.handle_response(response, owner, repo).await?;

        response
            .json::<Repository>()
            .await
            .map_err(|e| GitHubError::InvalidResponse { reason: e.to_string() }.into())
    }

    pub async fn get_branch(&self, owner: &str, repo: &str, branch: &str) -> Result<Branch> {
        validate_github_owner(owner)?;
        validate_github_repo(repo)?;
        validate_git_branch(branch)?;

        let encoded_owner = urlencoding::encode(owner);
        let encoded_repo = urlencoding::encode(repo);
        let encoded_branch = urlencoding::encode(branch);
        let url = format!("{GITHUB_API_BASE}/repos/{encoded_owner}/{encoded_repo}/branches/{encoded_branch}");

        let response = self.request_with_retry("get_branch", || {
            self.client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
        }).await?;

        let status = response.status();

        if status == StatusCode::NOT_FOUND {
            return Err(GitHubError::BranchNotFound {
                owner: owner.to_string(),
                repo: repo.to_string(),
                branch: branch.to_string(),
            }.into());
        }

        let response = self.handle_response(response, owner, repo).await?;

        response
            .json::<Branch>()
            .await
            .map_err(|e| GitHubError::InvalidResponse { reason: e.to_string() }.into())
    }

    pub async fn get_commit(&self, owner: &str, repo: &str, sha: &str) -> Result<Commit> {
        validate_github_owner(owner)?;
        validate_github_repo(repo)?;

        if sha.is_empty() {
            return Err(crate::error::ValidationError::Empty { field: "sha" }.into());
        }

        let encoded_owner = urlencoding::encode(owner);
        let encoded_repo = urlencoding::encode(repo);
        let encoded_sha = urlencoding::encode(sha);
        let url = format!("{GITHUB_API_BASE}/repos/{encoded_owner}/{encoded_repo}/commits/{encoded_sha}");

        let response = self.request_with_retry("get_commit", || {
            self.client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
        }).await?;

        let response = self.handle_response(response, owner, repo).await?;

        #[derive(Deserialize)]
        struct CommitResponse {
            sha: String,
            commit: CommitInner,
        }

        #[derive(Deserialize)]
        struct CommitInner {
            message: String,
            author: Option<CommitAuthor>,
        }

        let resp = response
            .json::<CommitResponse>()
            .await
            .map_err(|e| GitHubError::InvalidResponse { reason: e.to_string() })?;

        Ok(Commit {
            sha: resp.sha,
            message: resp.commit.message,
            author: resp.commit.author,
        })
    }

    pub async fn download_archive(&self, owner: &str, repo: &str, git_ref: &str) -> Result<Vec<u8>> {
        validate_github_owner(owner)?;
        validate_github_repo(repo)?;

        if git_ref.is_empty() {
            return Err(crate::error::ValidationError::Empty { field: "git_ref" }.into());
        }

        let encoded_owner = urlencoding::encode(owner);
        let encoded_repo = urlencoding::encode(repo);
        let encoded_ref = urlencoding::encode(git_ref);
        let url = format!("{GITHUB_API_BASE}/repos/{encoded_owner}/{encoded_repo}/tarball/{encoded_ref}");

        let response = self.request_with_retry("download_archive", || {
            self.client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .send()
        }).await?;

        let response = self.handle_response(response, owner, repo).await?;

        if let Some(content_length) = response.content_length() {
            if content_length as usize > ARCHIVE_SIZE_LIMIT_BYTES {
                return Err(crate::error::StorageError::SizeLimitExceeded {
                    size_bytes: content_length,
                    limit_bytes: ARCHIVE_SIZE_LIMIT_BYTES as u64,
                }.into());
            }
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| GitHubError::RequestFailed {
                operation: "download_archive",
                reason: e.to_string(),
            })?;

        if bytes.len() > ARCHIVE_SIZE_LIMIT_BYTES {
            return Err(crate::error::StorageError::SizeLimitExceeded {
                size_bytes: bytes.len() as u64,
                limit_bytes: ARCHIVE_SIZE_LIMIT_BYTES as u64,
            }.into());
        }

        Ok(bytes.to_vec())
    }

    pub fn parse_webhook_event(event_type: &str) -> WebhookEvent {
        match event_type {
            "push" => WebhookEvent::Push,
            "pull_request" => WebhookEvent::PullRequest,
            "release" => WebhookEvent::Release,
            _ => WebhookEvent::Unknown,
        }
    }

    pub fn verify_webhook_signature(secret: &str, signature: &str, payload: &[u8]) -> Result<bool> {
        use std::fmt::Write;

        if secret.is_empty() {
            return Err(crate::error::ValidationError::Empty { field: "secret" }.into());
        }

        if signature.is_empty() {
            return Err(crate::error::ValidationError::Empty { field: "signature" }.into());
        }

        let signature = signature.strip_prefix("sha256=").unwrap_or(signature);

        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let tag = ring::hmac::sign(&key, payload);

        let mut expected = String::with_capacity(64);
        for byte in tag.as_ref() {
            write!(&mut expected, "{:02x}", byte).expect("write to string");
        }

        Ok(constant_time_compare(expected.as_bytes(), signature.as_bytes()))
    }
}

fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }

    result == 0
}
