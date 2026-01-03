use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::GitHubConfig;
use crate::error::{AppError, Result};

const GITHUB_API_BASE: &str = "https://api.github.com";
const TIMEOUT_SECONDS: u64 = 30;

pub struct GitHubProvider {
    client: Client,
    token:  String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id:            u64,
    pub name:          String,
    pub full_name:     String,
    pub default_branch: String,
    pub clone_url:     String,
    pub private:       bool,
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
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECONDS))
            .user_agent("rustdeploy/0.1.0")
            .build()
            .map_err(|e| AppError::GitHub(format!("failed to create client: {e}")))?;

        Ok(Self {
            client,
            token: config.token.clone(),
        })
    }

    pub async fn get_repository(&self, owner: &str, repo: &str) -> Result<Repository> {
        if owner.is_empty() {
            return Err(AppError::Validation("owner cannot be empty".into()));
        }

        if repo.is_empty() {
            return Err(AppError::Validation("repo cannot be empty".into()));
        }

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}");

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| AppError::GitHub(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::GitHub(format!("api error {status}: {body}")));
        }

        let repository = response
            .json::<Repository>()
            .await
            .map_err(|e| AppError::GitHub(format!("parse failed: {e}")))?;

        Ok(repository)
    }

    pub async fn get_branch(&self, owner: &str, repo: &str, branch: &str) -> Result<Branch> {
        if owner.is_empty() {
            return Err(AppError::Validation("owner cannot be empty".into()));
        }

        if repo.is_empty() {
            return Err(AppError::Validation("repo cannot be empty".into()));
        }

        if branch.is_empty() {
            return Err(AppError::Validation("branch cannot be empty".into()));
        }

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/branches/{branch}");

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| AppError::GitHub(format!("request failed: {e}")))?;

        if response.status().as_u16() == 404 {
            return Err(AppError::NotFound(format!("branch {branch} not found")));
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::GitHub(format!("api error {status}: {body}")));
        }

        let branch_info = response
            .json::<Branch>()
            .await
            .map_err(|e| AppError::GitHub(format!("parse failed: {e}")))?;

        Ok(branch_info)
    }

    pub async fn get_commit(&self, owner: &str, repo: &str, sha: &str) -> Result<Commit> {
        if owner.is_empty() {
            return Err(AppError::Validation("owner cannot be empty".into()));
        }

        if repo.is_empty() {
            return Err(AppError::Validation("repo cannot be empty".into()));
        }

        if sha.is_empty() {
            return Err(AppError::Validation("sha cannot be empty".into()));
        }

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/commits/{sha}");

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| AppError::GitHub(format!("request failed: {e}")))?;

        if response.status().as_u16() == 404 {
            return Err(AppError::NotFound(format!("commit {sha} not found")));
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::GitHub(format!("api error {status}: {body}")));
        }

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
            .map_err(|e| AppError::GitHub(format!("parse failed: {e}")))?;

        Ok(Commit {
            sha: resp.sha,
            message: resp.commit.message,
            author: resp.commit.author,
        })
    }

    pub async fn download_archive(
        &self,
        owner: &str,
        repo: &str,
        git_ref: &str,
    ) -> Result<Vec<u8>> {
        if owner.is_empty() {
            return Err(AppError::Validation("owner cannot be empty".into()));
        }

        if repo.is_empty() {
            return Err(AppError::Validation("repo cannot be empty".into()));
        }

        if git_ref.is_empty() {
            return Err(AppError::Validation("ref cannot be empty".into()));
        }

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/tarball/{git_ref}");

        let response = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| AppError::GitHub(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AppError::GitHub(format!("download failed: {status}")));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| AppError::GitHub(format!("read failed: {e}")))?
            .to_vec();

        Ok(bytes)
    }

    pub fn parse_webhook_event(event_type: &str) -> WebhookEvent {
        match event_type {
            "push" => WebhookEvent::Push,
            "pull_request" => WebhookEvent::PullRequest,
            "release" => WebhookEvent::Release,
            _ => WebhookEvent::Unknown,
        }
    }

    pub fn verify_webhook_signature(
        secret: &str,
        signature: &str,
        payload: &[u8],
    ) -> Result<bool> {
        use std::io::Write;

        if secret.is_empty() {
            return Err(AppError::Validation("secret cannot be empty".into()));
        }

        if signature.is_empty() {
            return Err(AppError::Validation("signature cannot be empty".into()));
        }

        let signature = signature.strip_prefix("sha256=").unwrap_or(signature);

        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let tag = ring::hmac::sign(&key, payload);

        let mut expected = String::new();
        for byte in tag.as_ref() {
            write!(&mut expected, "{:02x}", byte).unwrap();
        }

        Ok(expected == signature)
    }
}
