use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, Result};
use crate::providers::{GitHubProvider, CloudflareProvider, EmailProvider};
use crate::storage::S3Storage;

const DEPLOYMENT_TIMEOUT_SECONDS: u64 = 600;

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

impl DeploymentState {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Deployed | Self::Failed | Self::Cancelled)
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
        if config.project_name.is_empty() {
            return Err(AppError::Validation("project_name cannot be empty".into()));
        }

        if config.owner.is_empty() {
            return Err(AppError::Validation("owner cannot be empty".into()));
        }

        if config.repo.is_empty() {
            return Err(AppError::Validation("repo cannot be empty".into()));
        }

        if config.branch.is_empty() {
            return Err(AppError::Validation("branch cannot be empty".into()));
        }

        let now = Utc::now();

        Ok(Self {
            id:         Uuid::new_v4(),
            config,
            state:      DeploymentState::Pending,
            result:     None,
            created_at: now,
            updated_at: now,
            logs:       Vec::new(),
        })
    }

    fn transition(&mut self, next_state: DeploymentState) -> Result<()> {
        if !self.state.can_transition_to(next_state) {
            return Err(AppError::Deployment(format!(
                "invalid state transition: {:?} -> {:?}",
                self.state, next_state
            )));
        }

        self.state = next_state;
        self.updated_at = Utc::now();

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

        let result = self.execute_pipeline(github, storage, cloudflare).await;

        let duration_seconds = start_time.elapsed().as_secs();

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

        let commit_sha = self.clone_repository(github).await?;

        self.transition(DeploymentState::Building)?;
        self.log_info(format!("Building from commit {commit_sha}..."));

        let archive = self.build_project(github, &commit_sha).await?;

        self.transition(DeploymentState::Uploading)?;
        self.log_info("Uploading artifacts...");

        let artifact_key = self.upload_artifacts(storage, &archive).await?;

        self.transition(DeploymentState::Configuring)?;
        self.log_info("Configuring DNS...");

        let url = self.configure_dns(cloudflare, &artifact_key).await?;

        self.transition(DeploymentState::Deployed)?;
        self.log_info(format!("Deployed successfully to {url}"));

        Ok(url)
    }

    async fn clone_repository(&mut self, github: &GitHubProvider) -> Result<String> {
        let branch = github.get_branch(
            &self.config.owner,
            &self.config.repo,
            &self.config.branch,
        ).await?;

        let commit_sha = self.config.commit_sha
            .clone()
            .unwrap_or(branch.commit.sha);

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
        ).await?;

        if archive.is_empty() {
            self.transition(DeploymentState::Failed)?;
            self.log_error("Empty archive received");
            return Err(AppError::Deployment("empty archive".into()));
        }

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

        storage.upload(&key, archive.to_vec(), Some("application/gzip")).await?;

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

        let existing = cloudflare.list_dns_records(Some(&domain)).await?;

        if existing.is_empty() {
            let record = crate::providers::cloudflare::CreateDnsRecord {
                type_:   "CNAME".to_string(),
                name:    domain.clone(),
                content: "origin.example.com".to_string(),
                ttl:     1,
                proxied: true,
            };

            cloudflare.create_dns_record(record).await?;
        }

        Ok(format!("https://{domain}"))
    }

    pub fn cancel(&mut self) -> Result<()> {
        if self.state.is_terminal() {
            return Err(AppError::Deployment(
                "cannot cancel terminal deployment".into()
            ));
        }

        self.transition(DeploymentState::Cancelled)?;
        self.log_info("Deployment cancelled");

        Ok(())
    }

    pub fn fail(&mut self, error: &str) -> Result<()> {
        if self.state.is_terminal() {
            return Err(AppError::Deployment(
                "cannot fail terminal deployment".into()
            ));
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

    pub fn enqueue(&mut self, deployment: Deployment) -> Result<()> {
        if self.deployments.len() >= self.capacity {
            return Err(AppError::Deployment("queue is full".into()));
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

    pub fn cancel(&mut self, id: Uuid) -> Result<()> {
        let deployment = self.deployments
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| AppError::NotFound("deployment not found".into()))?;

        deployment.cancel()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Deployment> {
        self.deployments.iter()
    }
}
