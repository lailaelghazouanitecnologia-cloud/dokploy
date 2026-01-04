use std::fmt;
use std::time::Duration;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use uuid::Uuid;

use crate::error::{DeploymentError, Result};
use crate::providers::{GitHubProvider, CloudflareProvider, EmailProvider, CreateDnsRecord};
use crate::storage::S3Storage;
use crate::validation::{
    validate_project_name, validate_github_owner,
    validate_github_repo, validate_git_branch,
    validate_email, validate_domain,
};

const DEPLOYMENT_TIMEOUT_SECONDS: u64 = 600;
const CLONE_TIMEOUT_SECONDS: u64 = 120;
const BUILD_TIMEOUT_SECONDS: u64 = 300;
const UPLOAD_TIMEOUT_SECONDS: u64 = 120;
const CONFIG_TIMEOUT_SECONDS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentState {
    Pending,
    Cloning,
    Building,
    Uploading,
    Configuring,
    Deployed,
    Failed,
    Cancelled,
}

impl fmt::Display for DeploymentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Cloning => write!(f, "cloning"),
            Self::Building => write!(f, "building"),
            Self::Uploading => write!(f, "uploading"),
            Self::Configuring => write!(f, "configuring"),
            Self::Deployed => write!(f, "deployed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl DeploymentState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Deployed | Self::Failed | Self::Cancelled)
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Cloning | Self::Building | Self::Uploading | Self::Configuring)
    }

    fn can_transition_to(&self, next: DeploymentState) -> bool {
        match (self, next) {
            (Self::Pending, Self::Cloning)     => true,
            (Self::Pending, Self::Cancelled)   => true,
            (Self::Pending, Self::Failed)      => true,

            (Self::Cloning, Self::Building)    => true,
            (Self::Cloning, Self::Failed)      => true,
            (Self::Cloning, Self::Cancelled)   => true,

            (Self::Building, Self::Uploading)  => true,
            (Self::Building, Self::Failed)     => true,
            (Self::Building, Self::Cancelled)  => true,

            (Self::Uploading, Self::Configuring) => true,
            (Self::Uploading, Self::Failed)    => true,
            (Self::Uploading, Self::Cancelled) => true,

            (Self::Configuring, Self::Deployed) => true,
            (Self::Configuring, Self::Failed)  => true,
            (Self::Configuring, Self::Cancelled) => true,

            _ => false,
        }
    }

    #[allow(dead_code)]
    fn timeout_seconds(&self) -> u64 {
        match self {
            Self::Cloning => CLONE_TIMEOUT_SECONDS,
            Self::Building => BUILD_TIMEOUT_SECONDS,
            Self::Uploading => UPLOAD_TIMEOUT_SECONDS,
            Self::Configuring => CONFIG_TIMEOUT_SECONDS,
            _ => DEPLOYMENT_TIMEOUT_SECONDS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    pub project_name: String,
    pub owner:        String,
    pub repo:         String,
    pub branch:       String,
    pub commit_sha:   Option<String>,
    pub domain:       Option<String>,
    pub notify_email: Option<String>,
}

impl DeploymentConfig {
    pub fn validate(&self) -> Result<()> {
        validate_project_name(&self.project_name)?;
        validate_github_owner(&self.owner)?;
        validate_github_repo(&self.repo)?;
        validate_git_branch(&self.branch)?;

        if let Some(ref email) = self.notify_email {
            validate_email(email)?;
        }

        if let Some(ref domain) = self.domain {
            validate_domain(domain)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentResult {
    pub url:              Option<String>,
    pub duration_seconds: u64,
    pub error:            Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id:         Uuid,
    pub config:     DeploymentConfig,
    pub state:      DeploymentState,
    pub result:     Option<DeploymentResult>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub logs:       Vec<DeploymentLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentLog {
    pub timestamp: DateTime<Utc>,
    pub level:     LogLevel,
    pub message:   String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl Deployment {
    pub fn new(config: DeploymentConfig) -> Result<Self> {
        config.validate()?;

        let now = Utc::now();

        Ok(Self {
            id:         Uuid::new_v4(),
            config,
            state:      DeploymentState::Pending,
            result:     None,
            created_at: now,
            updated_at: now,
            started_at: None,
            logs:       Vec::new(),
        })
    }

    fn transition(&mut self, next_state: DeploymentState) -> Result<()> {
        if !self.state.can_transition_to(next_state) {
            return Err(DeploymentError::InvalidTransition {
                from: self.state.to_string(),
                to: next_state.to_string(),
            }.into());
        }

        self.state = next_state;
        self.updated_at = Utc::now();

        if next_state.is_active() && self.started_at.is_none() {
            self.started_at = Some(Utc::now());
        }

        Ok(())
    }

    fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.logs.push(DeploymentLog {
            timestamp: Utc::now(),
            level,
            message:   message.into(),
        });
    }

    fn log_info(&mut self, message: impl Into<String>) {
        self.log(LogLevel::Info, message);
    }

    #[allow(dead_code)]
    fn log_warn(&mut self, message: impl Into<String>) {
        self.log(LogLevel::Warn, message);
    }

    fn log_error(&mut self, message: impl Into<String>) {
        self.log(LogLevel::Error, message);
    }

    pub async fn execute(
        &mut self,
        github:     &GitHubProvider,
        storage:    &S3Storage,
        cloudflare: &CloudflareProvider,
        email:      &EmailProvider,
    ) -> Result<()> {
        let start_time = std::time::Instant::now();

        if let Some(ref notify) = self.config.notify_email {
            let _ = email.send_deployment_started(
                notify,
                &self.config.project_name,
                &self.config.branch,
                self.config.commit_sha.as_deref().unwrap_or("HEAD"),
            ).await;
        }

        let pipeline_timeout = Duration::from_secs(DEPLOYMENT_TIMEOUT_SECONDS);
        let pipeline_result = timeout(
            pipeline_timeout,
            self.execute_pipeline(github, storage, cloudflare)
        ).await;

        let duration_seconds = start_time.elapsed().as_secs();

        let result = match pipeline_result {
            Ok(inner_result) => inner_result,
            Err(_) => {
                self.log_error(format!("deployment timed out after {}s", DEPLOYMENT_TIMEOUT_SECONDS));
                let _ = self.transition(DeploymentState::Failed);
                Err(DeploymentError::Timeout {
                    seconds: DEPLOYMENT_TIMEOUT_SECONDS,
                    state: self.state.to_string(),
                }.into())
            }
        };

        match result {
            Ok(url) => {
                self.result = Some(DeploymentResult {
                    url:              Some(url.clone()),
                    duration_seconds,
                    error:            None,
                });

                if let Some(ref notify) = self.config.notify_email {
                    let _ = email.send_deployment_succeeded(
                        notify,
                        &self.config.project_name,
                        &self.config.branch,
                        &url,
                        duration_seconds,
                    ).await;
                }

                Ok(())
            }
            Err(e) => {
                self.result = Some(DeploymentResult {
                    url:              None,
                    duration_seconds,
                    error:            Some(e.to_string()),
                });

                if let Some(ref notify) = self.config.notify_email {
                    let _ = email.send_deployment_failed(
                        notify,
                        &self.config.project_name,
                        &self.config.branch,
                        &e.to_string(),
                    ).await;
                }

                Err(e)
            }
        }
    }

    async fn execute_pipeline(
        &mut self,
        github:     &GitHubProvider,
        storage:    &S3Storage,
        cloudflare: &CloudflareProvider,
    ) -> Result<String> {
        self.transition(DeploymentState::Cloning)?;
        self.log_info("Cloning repository...");

        let clone_timeout = Duration::from_secs(CLONE_TIMEOUT_SECONDS);
        let commit_sha = timeout(clone_timeout, self.clone_repository(github))
            .await
            .map_err(|_| {
                self.log_error(format!("clone timed out after {}s", CLONE_TIMEOUT_SECONDS));
                DeploymentError::Timeout {
                    seconds: CLONE_TIMEOUT_SECONDS,
                    state: "cloning".to_string(),
                }
            })??;

        self.transition(DeploymentState::Building)?;
        self.log_info(format!("Building from commit {commit_sha}..."));

        let build_timeout = Duration::from_secs(BUILD_TIMEOUT_SECONDS);
        let archive = timeout(build_timeout, self.build_project(github, &commit_sha))
            .await
            .map_err(|_| {
                self.log_error(format!("build timed out after {}s", BUILD_TIMEOUT_SECONDS));
                DeploymentError::Timeout {
                    seconds: BUILD_TIMEOUT_SECONDS,
                    state: "building".to_string(),
                }
            })??;

        self.transition(DeploymentState::Uploading)?;
        self.log_info("Uploading artifacts...");

        let upload_timeout = Duration::from_secs(UPLOAD_TIMEOUT_SECONDS);
        let artifact_key = timeout(upload_timeout, self.upload_artifacts(storage, &archive))
            .await
            .map_err(|_| {
                self.log_error(format!("upload timed out after {}s", UPLOAD_TIMEOUT_SECONDS));
                DeploymentError::Timeout {
                    seconds: UPLOAD_TIMEOUT_SECONDS,
                    state: "uploading".to_string(),
                }
            })??;

        self.transition(DeploymentState::Configuring)?;
        self.log_info("Configuring DNS...");

        let config_timeout = Duration::from_secs(CONFIG_TIMEOUT_SECONDS);
        let url = timeout(config_timeout, self.configure_dns(cloudflare, &artifact_key))
            .await
            .map_err(|_| {
                self.log_error(format!("configuration timed out after {}s", CONFIG_TIMEOUT_SECONDS));
                DeploymentError::Timeout {
                    seconds: CONFIG_TIMEOUT_SECONDS,
                    state: "configuring".to_string(),
                }
            })??;

        self.transition(DeploymentState::Deployed)?;
        self.log_info(format!("Deployed successfully to {url}"));

        Ok(url)
    }

    async fn clone_repository(&mut self, github: &GitHubProvider) -> Result<String> {
        let branch = github.get_branch(
            &self.config.owner,
            &self.config.repo,
            &self.config.branch,
        ).await.map_err(|e| {
            DeploymentError::CloneFailed {
                owner: self.config.owner.clone(),
                repo: self.config.repo.clone(),
                branch: self.config.branch.clone(),
                reason: e.to_string(),
            }
        })?;

        let commit_sha = self.config.commit_sha
            .clone()
            .unwrap_or(branch.commit.sha);

        self.log_info(format!("Resolved commit: {commit_sha}"));

        Ok(commit_sha)
    }

    async fn build_project(
        &mut self,
        github: &GitHubProvider,
        commit_sha: &str,
    ) -> Result<Vec<u8>> {
        let archive = github.download_archive(
            &self.config.owner,
            &self.config.repo,
            commit_sha,
        ).await.map_err(|e| {
            DeploymentError::BuildFailed {
                reason: format!("failed to download archive: {e}"),
            }
        })?;

        if archive.is_empty() {
            self.transition(DeploymentState::Failed)?;
            self.log_error("Empty archive received");
            return Err(DeploymentError::BuildFailed {
                reason: "empty archive".to_string(),
            }.into());
        }

        self.log_info(format!("Downloaded {} bytes", archive.len()));

        Ok(archive)
    }

    async fn upload_artifacts(
        &mut self,
        storage: &S3Storage,
        archive: &[u8],
    ) -> Result<String> {
        let key = format!(
            "deployments/{}/{}/{}",
            self.config.project_name,
            self.id,
            Utc::now().format("%Y%m%d%H%M%S")
        );

        storage.upload(&key, archive.to_vec(), Some("application/gzip"))
            .await
            .map_err(|e| DeploymentError::UploadFailed {
                reason: e.to_string(),
            })?;

        self.log_info(format!("Uploaded to {key}"));

        Ok(key)
    }

    async fn configure_dns(
        &mut self,
        cloudflare: &CloudflareProvider,
        _artifact_key: &str,
    ) -> Result<String> {
        let domain = self.config.domain
            .clone()
            .unwrap_or_else(|| format!("{}.example.com", self.config.project_name));

        let existing = cloudflare.list_dns_records(Some(&domain)).await
            .map_err(|e| DeploymentError::ConfigFailed {
                reason: format!("failed to list DNS records: {e}"),
            })?;

        if existing.is_empty() {
            let record = CreateDnsRecord {
                type_:   "CNAME".to_string(),
                name:    domain.clone(),
                content: "origin.example.com".to_string(),
                ttl:     1,
                proxied: true,
            };

            cloudflare.create_dns_record(record).await
                .map_err(|e| DeploymentError::ConfigFailed {
                    reason: format!("failed to create DNS record: {e}"),
                })?;

            self.log_info(format!("Created DNS record for {domain}"));
        } else {
            self.log_info(format!("DNS record already exists for {domain}"));
        }

        Ok(format!("https://{domain}"))
    }

    pub fn cancel(&mut self) -> Result<()> {
        if self.state.is_terminal() {
            return Err(DeploymentError::AlreadyTerminal {
                state: self.state.to_string(),
            }.into());
        }

        self.transition(DeploymentState::Cancelled)?;
        self.log_info("Deployment cancelled");

        Ok(())
    }

    pub fn fail(&mut self, error: &str) -> Result<()> {
        if self.state.is_terminal() {
            return Err(DeploymentError::AlreadyTerminal {
                state: self.state.to_string(),
            }.into());
        }

        self.transition(DeploymentState::Failed)?;
        self.log_error(error);

        self.result = Some(DeploymentResult {
            url:              None,
            duration_seconds: 0,
            error:            Some(error.to_string()),
        });

        Ok(())
    }

    pub fn elapsed_seconds(&self) -> Option<u64> {
        self.started_at.map(|started| {
            let now = Utc::now();
            (now - started).num_seconds().max(0) as u64
        })
    }
}

pub struct DeploymentQueue {
    deployments: Vec<Deployment>,
    capacity:    usize,
}

impl DeploymentQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            deployments: Vec::with_capacity(capacity),
            capacity,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn enqueue(&mut self, deployment: Deployment) -> Result<()> {
        if self.deployments.len() >= self.capacity {
            return Err(DeploymentError::QueueFull {
                current: self.deployments.len(),
                capacity: self.capacity,
            }.into());
        }

        self.deployments.push(deployment);
        Ok(())
    }

    pub fn dequeue(&mut self) -> Option<Deployment> {
        if self.deployments.is_empty() {
            return None;
        }

        Some(self.deployments.remove(0))
    }

    pub fn len(&self) -> usize {
        self.deployments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.deployments.is_empty()
    }

    pub fn find(&self, id: Uuid) -> Option<&Deployment> {
        self.deployments.iter().find(|d| d.id == id)
    }

    pub fn find_mut(&mut self, id: Uuid) -> Option<&mut Deployment> {
        self.deployments.iter_mut().find(|d| d.id == id)
    }

    pub fn cancel(&mut self, id: Uuid) -> Result<()> {
        let deployment = self.deployments
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| DeploymentError::NotFound { id: id.to_string() })?;

        deployment.cancel()
    }

    pub fn remove(&mut self, id: Uuid) -> Option<Deployment> {
        let index = self.deployments.iter().position(|d| d.id == id)?;
        Some(self.deployments.remove(index))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Deployment> {
        self.deployments.iter()
    }

    pub fn pending(&self) -> impl Iterator<Item = &Deployment> {
        self.deployments.iter().filter(|d| d.state == DeploymentState::Pending)
    }

    pub fn active(&self) -> impl Iterator<Item = &Deployment> {
        self.deployments.iter().filter(|d| d.state.is_active())
    }

    pub fn cleanup_terminal(&mut self) -> Vec<Deployment> {
        let mut removed = Vec::new();
        self.deployments.retain(|d| {
            if d.state.is_terminal() {
                removed.push(d.clone());
                false
            } else {
                true
            }
        });
        removed
    }
}
