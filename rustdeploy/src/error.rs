use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("missing environment variable: {var}")]
    MissingEnvVar { var: &'static str },

    #[error("invalid value for {var}: {reason}")]
    InvalidValue { var: &'static str, reason: String },

    #[error("validation failed: {0}")]
    Validation(String),
}

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("bucket not accessible: {bucket}")]
    BucketNotAccessible { bucket: String },

    #[error("upload failed for key {key}: {reason}")]
    UploadFailed { key: String, reason: String },

    #[error("download failed for key {key}: {reason}")]
    DownloadFailed { key: String, reason: String },

    #[error("object not found: {key}")]
    NotFound { key: String },

    #[error("size limit exceeded: {size_bytes} > {limit_bytes}")]
    SizeLimitExceeded { size_bytes: u64, limit_bytes: u64 },

    #[error("invalid key: {reason}")]
    InvalidKey { key: String, reason: &'static str },
}

#[derive(Error, Debug)]
pub enum GitHubError {
    #[error("repository not found: {owner}/{repo}")]
    RepoNotFound { owner: String, repo: String },

    #[error("branch not found: {branch} in {owner}/{repo}")]
    BranchNotFound { owner: String, repo: String, branch: String },

    #[error("rate limited, retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("authentication failed")]
    AuthFailed,

    #[error("request failed: {operation}")]
    RequestFailed { operation: &'static str, reason: String },

    #[error("invalid response: {reason}")]
    InvalidResponse { reason: String },

    #[error("invalid identifier: {value} ({reason})")]
    InvalidIdentifier { value: String, reason: &'static str },
}

#[derive(Error, Debug)]
pub enum CloudflareError {
    #[error("dns record not found: {name}")]
    RecordNotFound { name: String },

    #[error("api error {code}: {message}")]
    ApiError { code: u32, message: String },

    #[error("request failed: {operation}")]
    RequestFailed { operation: &'static str, reason: String },

    #[error("invalid record type: {type_}")]
    InvalidRecordType { type_: String },
}

#[derive(Error, Debug)]
pub enum EmailError {
    #[error("smtp connection failed: {host}:{port}")]
    ConnectionFailed { host: String, port: u16, reason: String },

    #[error("send failed to {to}: {reason}")]
    SendFailed { to: String, reason: String },

    #[error("invalid address: {address}")]
    InvalidAddress { address: String },

    #[error("message too large: {size_bytes} > {limit_bytes}")]
    MessageTooLarge { size_bytes: usize, limit_bytes: usize },
}

#[derive(Error, Debug)]
pub enum GroqError {
    #[error("request failed: {reason}")]
    RequestFailed { reason: String },

    #[error("api error {status}: {message}")]
    ApiError { status: u16, message: String },

    #[error("no response generated")]
    EmptyResponse,

    #[error("prompt too large: {length} > {limit}")]
    PromptTooLarge { length: usize, limit: usize },

    #[error("invalid role: {role}")]
    InvalidRole { role: String },
}

#[derive(Error, Debug)]
pub enum DeploymentError {
    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("deployment timed out after {seconds}s in state {state}")]
    Timeout { seconds: u64, state: String },

    #[error("clone failed for {owner}/{repo}:{branch}: {reason}")]
    CloneFailed { owner: String, repo: String, branch: String, reason: String },

    #[error("build failed: {reason}")]
    BuildFailed { reason: String },

    #[error("upload failed: {reason}")]
    UploadFailed { reason: String },

    #[error("configuration failed: {reason}")]
    ConfigFailed { reason: String },

    #[error("queue full: {current}/{capacity}")]
    QueueFull { current: usize, capacity: usize },

    #[error("deployment not found: {id}")]
    NotFound { id: String },

    #[error("already in terminal state: {state}")]
    AlreadyTerminal { state: String },
}

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("{field}: {reason}")]
    Field { field: &'static str, reason: String },

    #[error("{field} cannot be empty")]
    Empty { field: &'static str },

    #[error("{field} too long: {length} > {max}")]
    TooLong { field: &'static str, length: usize, max: usize },

    #[error("{field} too short: {length} < {min}")]
    TooShort { field: &'static str, length: usize, min: usize },

    #[error("{field} invalid format: {reason}")]
    InvalidFormat { field: &'static str, reason: &'static str },

    #[error("{field}: path traversal attempt")]
    PathTraversal { field: &'static str },

    #[error("{field}: invalid email address")]
    InvalidEmail { field: &'static str },
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error(transparent)]
    GitHub(#[from] GitHubError),

    #[error(transparent)]
    Cloudflare(#[from] CloudflareError),

    #[error(transparent)]
    Email(#[from] EmailError),

    #[error(transparent)]
    Groq(#[from] GroqError),

    #[error(transparent)]
    Deployment(#[from] DeploymentError),

    #[error(transparent)]
    Validation(#[from] ValidationError),

    #[error("unauthorized")]
    Unauthorized,

    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Validation(_) => 400,
            Self::Unauthorized => 401,
            Self::Config(_) => 500,
            Self::Internal(_) => 500,

            Self::Storage(e) => match e {
                StorageError::NotFound { .. } => 404,
                StorageError::InvalidKey { .. } => 400,
                StorageError::SizeLimitExceeded { .. } => 413,
                _ => 502,
            },

            Self::GitHub(e) => match e {
                GitHubError::RepoNotFound { .. } => 404,
                GitHubError::BranchNotFound { .. } => 404,
                GitHubError::AuthFailed => 401,
                GitHubError::RateLimited { .. } => 429,
                GitHubError::InvalidIdentifier { .. } => 400,
                _ => 502,
            },

            Self::Cloudflare(e) => match e {
                CloudflareError::RecordNotFound { .. } => 404,
                CloudflareError::InvalidRecordType { .. } => 400,
                _ => 502,
            },

            Self::Email(e) => match e {
                EmailError::InvalidAddress { .. } => 400,
                EmailError::MessageTooLarge { .. } => 413,
                _ => 502,
            },

            Self::Groq(e) => match e {
                GroqError::PromptTooLarge { .. } => 413,
                GroqError::InvalidRole { .. } => 400,
                _ => 502,
            },

            Self::Deployment(e) => match e {
                DeploymentError::NotFound { .. } => 404,
                DeploymentError::QueueFull { .. } => 503,
                DeploymentError::InvalidTransition { .. } => 409,
                DeploymentError::AlreadyTerminal { .. } => 409,
                _ => 500,
            },
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Config(_) => "CONFIG_ERROR",
            Self::Storage(_) => "STORAGE_ERROR",
            Self::GitHub(_) => "GITHUB_ERROR",
            Self::Cloudflare(_) => "CLOUDFLARE_ERROR",
            Self::Email(_) => "EMAIL_ERROR",
            Self::Groq(_) => "GROQ_ERROR",
            Self::Deployment(_) => "DEPLOYMENT_ERROR",
            Self::Validation(_) => "VALIDATION_ERROR",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
