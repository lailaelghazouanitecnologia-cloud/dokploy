use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("github error: {0}")]
    GitHub(String),

    #[error("cloudflare error: {0}")]
    Cloudflare(String),

    #[error("email error: {0}")]
    Email(String),

    #[error("groq error: {0}")]
    Groq(String),

    #[error("deployment error: {0}")]
    Deployment(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

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
            Self::NotFound(_) => 404,
            Self::Config(_) => 500,
            Self::Storage(_) => 502,
            Self::GitHub(_) => 502,
            Self::Cloudflare(_) => 502,
            Self::Email(_) => 502,
            Self::Groq(_) => 502,
            Self::Deployment(_) => 500,
            Self::Internal(_) => 500,
        }
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
