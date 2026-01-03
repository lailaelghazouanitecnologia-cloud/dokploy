use crate::error::{AppError, Result};

const PORT_MIN: u16 = 1024;
const PORT_MAX: u16 = 65535;
const KEY_LENGTH_MIN: usize = 16;

#[derive(Debug, Clone)]
pub struct Config {
    pub server:     ServerConfig,
    pub s3:         S3Config,
    pub github:     GitHubConfig,
    pub cloudflare: CloudflareConfig,
    pub email:      EmailConfig,
    pub groq:       GroqConfig,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct S3Config {
    pub bucket:     String,
    pub region:     String,
    pub access_key: String,
    pub secret_key: String,
    pub endpoint:   Option<String>,
}

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    pub token:        String,
    pub webhook_secret: String,
}

#[derive(Debug, Clone)]
pub struct CloudflareConfig {
    pub api_token: String,
    pub zone_id:   String,
}

#[derive(Debug, Clone)]
pub struct EmailConfig {
    pub smtp_host:     String,
    pub smtp_port:     u16,
    pub smtp_user:     String,
    pub smtp_password: String,
    pub from_address:  String,
}

#[derive(Debug, Clone)]
pub struct GroqConfig {
    pub api_key:  String,
    pub model_id: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let config = Self {
            server: ServerConfig {
                host: get_env("SERVER_HOST")?,
                port: get_env_parse("SERVER_PORT")?,
            },
            s3: S3Config {
                bucket:     get_env("S3_BUCKET")?,
                region:     get_env("S3_REGION")?,
                access_key: get_env("S3_ACCESS_KEY")?,
                secret_key: get_env("S3_SECRET_KEY")?,
                endpoint:   get_env_optional("S3_ENDPOINT"),
            },
            github: GitHubConfig {
                token:          get_env("GITHUB_TOKEN")?,
                webhook_secret: get_env("GITHUB_WEBHOOK_SECRET")?,
            },
            cloudflare: CloudflareConfig {
                api_token: get_env("CLOUDFLARE_API_TOKEN")?,
                zone_id:   get_env("CLOUDFLARE_ZONE_ID")?,
            },
            email: EmailConfig {
                smtp_host:     get_env("SMTP_HOST")?,
                smtp_port:     get_env_parse("SMTP_PORT")?,
                smtp_user:     get_env("SMTP_USER")?,
                smtp_password: get_env("SMTP_PASSWORD")?,
                from_address:  get_env("EMAIL_FROM")?,
            },
            groq: GroqConfig {
                api_key:  get_env("GROQ_API_KEY")?,
                model_id: get_env_or("GROQ_MODEL", "llama-3.3-70b-versatile"),
            },
        };

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.server.port < PORT_MIN {
            return Err(AppError::Validation(format!(
                "port must be >= {PORT_MIN}"
            )));
        }

        if self.server.port > PORT_MAX {
            return Err(AppError::Validation(format!(
                "port must be <= {PORT_MAX}"
            )));
        }

        if self.s3.bucket.is_empty() {
            return Err(AppError::Validation("s3 bucket cannot be empty".into()));
        }

        if self.github.token.len() < KEY_LENGTH_MIN {
            return Err(AppError::Validation(format!(
                "github token must be >= {KEY_LENGTH_MIN} chars"
            )));
        }

        if self.cloudflare.api_token.len() < KEY_LENGTH_MIN {
            return Err(AppError::Validation(format!(
                "cloudflare token must be >= {KEY_LENGTH_MIN} chars"
            )));
        }

        if self.groq.api_key.len() < KEY_LENGTH_MIN {
            return Err(AppError::Validation(format!(
                "groq api key must be >= {KEY_LENGTH_MIN} chars"
            )));
        }

        if !self.email.from_address.contains('@') {
            return Err(AppError::Validation("invalid email from address".into()));
        }

        Ok(())
    }
}

fn get_env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| {
        AppError::Config(format!("missing env var: {key}"))
    })
}

fn get_env_optional(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn get_env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn get_env_parse<T: std::str::FromStr>(key: &str) -> Result<T> {
    let value = get_env(key)?;
    value.parse().map_err(|_| {
        AppError::Config(format!("invalid value for {key}"))
    })
}
