use once_cell::sync::Lazy;
use regex::Regex;

use crate::error::{ValidationError, Result};

pub const PROJECT_NAME_MAX: usize = 100;
pub const GITHUB_USERNAME_MAX: usize = 39;
pub const GITHUB_REPO_MAX: usize = 100;
pub const GIT_BRANCH_MAX: usize = 255;
pub const COMMIT_SHA_LENGTH: usize = 40;
pub const DOMAIN_MAX: usize = 253;
pub const EMAIL_MAX: usize = 254;
pub const S3_KEY_MAX: usize = 1024;

static SAFE_NAME_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z][a-zA-Z0-9_-]*$").unwrap()
});

static GITHUB_USERNAME_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?$").unwrap()
});

static GITHUB_REPO_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9._-]+$").unwrap()
});

static GIT_BRANCH_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9._/-]+$").unwrap()
});

static HEX_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-fA-F0-9]+$").unwrap()
});

static DOMAIN_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}$").unwrap()
});

static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap()
});

pub fn validate_not_empty(value: &str, field: &'static str) -> Result<()> {
    if value.is_empty() {
        return Err(ValidationError::Empty { field }.into());
    }
    Ok(())
}

pub fn validate_max_length(value: &str, max: usize, field: &'static str) -> Result<()> {
    if value.len() > max {
        return Err(ValidationError::TooLong {
            field,
            length: value.len(),
            max,
        }.into());
    }
    Ok(())
}

pub fn validate_min_length(value: &str, min: usize, field: &'static str) -> Result<()> {
    if value.len() < min {
        return Err(ValidationError::TooShort {
            field,
            length: value.len(),
            min,
        }.into());
    }
    Ok(())
}

pub fn validate_exact_length(value: &str, length: usize, field: &'static str) -> Result<()> {
    if value.len() != length {
        return Err(ValidationError::Field {
            field,
            reason: format!("must be exactly {} characters", length),
        }.into());
    }
    Ok(())
}

pub fn validate_no_path_traversal(value: &str, field: &'static str) -> Result<()> {
    if value.contains("..") {
        return Err(ValidationError::PathTraversal { field }.into());
    }

    if value.starts_with('/') {
        return Err(ValidationError::PathTraversal { field }.into());
    }

    if value.contains('\0') {
        return Err(ValidationError::PathTraversal { field }.into());
    }

    Ok(())
}

pub fn validate_project_name(value: &str) -> Result<()> {
    let field = "project_name";

    validate_not_empty(value, field)?;
    validate_max_length(value, PROJECT_NAME_MAX, field)?;

    if !SAFE_NAME_REGEX.is_match(value) {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "must start with letter and contain only alphanumeric, underscore, hyphen",
        }.into());
    }

    Ok(())
}

pub fn validate_github_owner(value: &str) -> Result<()> {
    let field = "owner";

    validate_not_empty(value, field)?;
    validate_max_length(value, GITHUB_USERNAME_MAX, field)?;

    if !GITHUB_USERNAME_REGEX.is_match(value) {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "invalid GitHub username format",
        }.into());
    }

    if value.contains("--") {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "cannot contain consecutive hyphens",
        }.into());
    }

    Ok(())
}

pub fn validate_github_repo(value: &str) -> Result<()> {
    let field = "repo";

    validate_not_empty(value, field)?;
    validate_max_length(value, GITHUB_REPO_MAX, field)?;

    if !GITHUB_REPO_REGEX.is_match(value) {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "invalid GitHub repository name format",
        }.into());
    }

    if value == "." || value == ".." {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "cannot be . or ..",
        }.into());
    }

    Ok(())
}

pub fn validate_git_branch(value: &str) -> Result<()> {
    let field = "branch";

    validate_not_empty(value, field)?;
    validate_max_length(value, GIT_BRANCH_MAX, field)?;

    if !GIT_BRANCH_REGEX.is_match(value) {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "invalid git branch name format",
        }.into());
    }

    if value.starts_with('-') || value.ends_with('.') || value.ends_with('/') {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "invalid branch name start/end character",
        }.into());
    }

    if value.contains("..") || value.contains("//") {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "cannot contain .. or //",
        }.into());
    }

    Ok(())
}

pub fn validate_commit_sha(value: &str) -> Result<()> {
    let field = "commit_sha";

    validate_exact_length(value, COMMIT_SHA_LENGTH, field)?;

    if !HEX_REGEX.is_match(value) {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "must be hexadecimal",
        }.into());
    }

    Ok(())
}

pub fn validate_domain(value: &str) -> Result<()> {
    let field = "domain";

    validate_not_empty(value, field)?;
    validate_max_length(value, DOMAIN_MAX, field)?;

    if !DOMAIN_REGEX.is_match(value) {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "invalid domain format",
        }.into());
    }

    Ok(())
}

pub fn validate_email(value: &str) -> Result<()> {
    let field = "email";

    validate_not_empty(value, field)?;
    validate_max_length(value, EMAIL_MAX, field)?;

    if !EMAIL_REGEX.is_match(value) {
        return Err(ValidationError::InvalidEmail { field }.into());
    }

    Ok(())
}

pub fn validate_s3_key(value: &str) -> Result<()> {
    let field = "s3_key";

    validate_not_empty(value, field)?;
    validate_max_length(value, S3_KEY_MAX, field)?;
    validate_no_path_traversal(value, field)?;

    if value.contains('\0') {
        return Err(ValidationError::InvalidFormat {
            field,
            reason: "cannot contain null bytes",
        }.into());
    }

    Ok(())
}

pub fn validate_port(value: u16) -> Result<()> {
    if value < 1 {
        return Err(ValidationError::Field {
            field: "port",
            reason: "must be >= 1".into(),
        }.into());
    }

    Ok(())
}

pub fn validate_port_unprivileged(value: u16) -> Result<()> {
    if value < 1024 {
        return Err(ValidationError::Field {
            field: "port",
            reason: "must be >= 1024 (unprivileged)".into(),
        }.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_project_name_valid() {
        assert!(validate_project_name("myproject").is_ok());
        assert!(validate_project_name("my-project").is_ok());
        assert!(validate_project_name("my_project_123").is_ok());
    }

    #[test]
    fn test_validate_project_name_invalid() {
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("123project").is_err());
        assert!(validate_project_name("-project").is_err());
        assert!(validate_project_name("project!").is_err());
    }

    #[test]
    fn test_validate_github_owner_valid() {
        assert!(validate_github_owner("octocat").is_ok());
        assert!(validate_github_owner("my-org").is_ok());
        assert!(validate_github_owner("a1").is_ok());
    }

    #[test]
    fn test_validate_github_owner_invalid() {
        assert!(validate_github_owner("").is_err());
        assert!(validate_github_owner("-invalid").is_err());
        assert!(validate_github_owner("invalid-").is_err());
        assert!(validate_github_owner("in--valid").is_err());
    }

    #[test]
    fn test_validate_no_path_traversal() {
        assert!(validate_no_path_traversal("valid/path", "test").is_ok());
        assert!(validate_no_path_traversal("../etc/passwd", "test").is_err());
        assert!(validate_no_path_traversal("/etc/passwd", "test").is_err());
    }

    #[test]
    fn test_validate_commit_sha() {
        assert!(validate_commit_sha("a".repeat(40).as_str()).is_ok());
        assert!(validate_commit_sha("abc123").is_err());
        assert!(validate_commit_sha("z".repeat(40).as_str()).is_err());
    }

    #[test]
    fn test_validate_email() {
        assert!(validate_email("user@example.com").is_ok());
        assert!(validate_email("user.name+tag@domain.org").is_ok());
        assert!(validate_email("invalid").is_err());
        assert!(validate_email("@domain.com").is_err());
    }
}
