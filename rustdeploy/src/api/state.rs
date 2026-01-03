use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::deployment::DeploymentQueue;
use crate::providers::{CloudflareProvider, EmailProvider, GitHubProvider, GroqProvider};
use crate::storage::S3Storage;

const QUEUE_CAPACITY: usize = 100;

pub struct AppState {
    pub config:     Config,
    pub storage:    S3Storage,
    pub github:     GitHubProvider,
    pub cloudflare: CloudflareProvider,
    pub email:      EmailProvider,
    pub groq:       GroqProvider,
    pub queue:      RwLock<DeploymentQueue>,
}

impl AppState {
    pub async fn new(config: Config) -> crate::error::Result<Arc<Self>> {
        let storage = S3Storage::new(&config.s3).await?;
        let github = GitHubProvider::new(&config.github)?;
        let cloudflare = CloudflareProvider::new(&config.cloudflare)?;
        let email = EmailProvider::new(&config.email).await?;
        let groq = GroqProvider::new(&config.groq)?;
        let queue = RwLock::new(DeploymentQueue::new(QUEUE_CAPACITY));

        Ok(Arc::new(Self {
            config,
            storage,
            github,
            cloudflare,
            email,
            groq,
            queue,
        }))
    }
}
