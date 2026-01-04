use crate::error::{ConfigError, Result};
use crate::validation::{validate_email, validate_port_unprivileged};

const KEY_LENGTH_MIN: usize = 16;
const API_KEY_LENGTH_MIN: usize = 32;
const API_KEY_LENGTH_MAX: usize = 256;

#[derive(Debug, Clone)]
pub struct Config {
    pub server:     ServerConfig,
    pub api:        ApiConfig,
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

#[derive(Clone)]
pub struct ApiConfig {
    api_key: String,
}

impl std::fmt::Debug for ApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiConfig")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl ApiConfig {
    pub fn verify_key(&self, candidate: &str) -> bool {
        use subtle::ConstantTimeEq;

        if candidate.len() != self.api_key.len() {
            return false;
        }

        self.api_key.as_bytes().ct_eq(candidate.as_bytes()).into()
    }

    pub fn key_hash(&self) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(self.api_key.as_bytes());
        format!("{:x}", hasher.finalize())[..16].to_string()
    }
}

#[derive(Clone)]
pub struct S3Config {
    pub bucket:     String,
    pub region:     String,
    pub access_key: String,
    pub secret_key: String,
    pub endpoint:   Option<String>,
}

impl std::fmt::Debug for S3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key", &"[REDACTED]")
            .field("secret_key", &"[REDACTED]")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

#[derive(Clone)]
pub struct GitHubConfig {
    pub token:          String,
    pub webhook_secret: Option<String>,
}

impl std::fmt::Debug for GitHubConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubConfig")
            .field("token", &"[REDACTED]")
            .field("webhook_secret", &self.webhook_secret.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone)]
pub struct CloudflareConfig {
    pub api_token: String,
    pub zone_id:   String,
}

impl std::fmt::Debug for CloudflareConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareConfig")
            .field("api_token", &"[REDACTED]")
            .field("zone_id", &self.zone_id)
            .finish()
    }
}

#[derive(Clone)]
pub struct EmailConfig {
    pub smtp_host:     String,
    pub smtp_port:     u16,
    pub smtp_user:     String,
    pub smtp_password: String,
    pub from_address:  String,
}

impl std::fmt::Debug for EmailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailConfig")
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_user", &"[REDACTED]")
            .field("smtp_password", &"[REDACTED]")
            .field("from_address", &self.from_address)
            .finish()
    }
}

#[derive(Clone)]
pub struct GroqConfig {
    pub api_key:  String,
    pub model_id: String,
}

impl std::fmt::Debug for GroqConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroqConfig")
            .field("api_key", &"[REDACTED]")
            .field("model_id", &self.model_id)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let config = Self {
            server: ServerConfig {
                host: get_env("SERVER_HOST")?,
                port: get_env_parse("SERVER_PORT")?,
            },
            api: ApiConfig {
                api_key: get_env("API_KEY")?,
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
                webhook_secret: get_env_optional("GITHUB_WEBHOOK_SECRET"),
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
        validate_port_unprivileged(self.server.port)?;

        if self.api.api_key.len() < API_KEY_LENGTH_MIN {
            return Err(ConfigError::InvalidValue {
                var:    "API_KEY",
                reason: format!("must be >= {API_KEY_LENGTH_MIN} chars"),
            }.into());
        }

        if self.api.api_key.len() > API_KEY_LENGTH_MAX {
            return Err(ConfigError::InvalidValue {
                var:    "API_KEY",
                reason: format!("must be <= {API_KEY_LENGTH_MAX} chars"),
            }.into());
        }

        if self.s3.bucket.is_empty() {
            return Err(ConfigError::InvalidValue {
                var:    "S3_BUCKET",
                reason: "cannot be empty".into(),
            }.into());
        }

        if self.github.token.len() < KEY_LENGTH_MIN {
            return Err(ConfigError::InvalidValue {
                var:    "GITHUB_TOKEN",
                reason: format!("must be >= {KEY_LENGTH_MIN} chars"),
            }.into());
        }

        if let Some(ref secret) = self.github.webhook_secret {
            if secret.len() < KEY_LENGTH_MIN {
                return Err(ConfigError::InvalidValue {
                    var:    "GITHUB_WEBHOOK_SECRET",
                    reason: format!("must be >= {KEY_LENGTH_MIN} chars"),
                }.into());
            }
        }

        if self.cloudflare.api_token.len() < KEY_LENGTH_MIN {
            return Err(ConfigError::InvalidValue {
                var:    "CLOUDFLARE_API_TOKEN",
                reason: format!("must be >= {KEY_LENGTH_MIN} chars"),
            }.into());
        }

        if self.cloudflare.zone_id.is_empty() {
            return Err(ConfigError::InvalidValue {
                var:    "CLOUDFLARE_ZONE_ID",
                reason: "cannot be empty".into(),
            }.into());
        }

        if self.groq.api_key.len() < KEY_LENGTH_MIN {
            return Err(ConfigError::InvalidValue {
                var:    "GROQ_API_KEY",
                reason: format!("must be >= {KEY_LENGTH_MIN} chars"),
            }.into());
        }

        validate_email(&self.email.from_address)?;
        validate_port_unprivileged(self.email.smtp_port)?;

        Ok(())
    }
}

fn get_env(key: &'static str) -> Result<String> {
    std::env::var(key).map_err(|_| {
        ConfigError::MissingEnvVar { var: key }.into()
    })
}

fn get_env_optional(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn get_env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn get_env_parse<T: std::str::FromStr>(key: &'static str) -> Result<T> {
    let value = get_env(key)?;
    value.parse().map_err(|_| {
        ConfigError::InvalidValue {
            var:    key,
            reason: "failed to parse".into(),
        }.into()
    })
}
