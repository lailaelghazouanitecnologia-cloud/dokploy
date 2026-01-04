use std::sync::Arc;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::deployment::{Deployment, DeploymentConfig, DeploymentState};
use crate::providers::GitHubProvider;

use super::state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    status:  String,
    version: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    error:   String,
    code:    u16,
}

#[derive(Deserialize)]
pub struct CreateDeploymentRequest {
    project_name: String,
    owner:        String,
    repo:         String,
    branch:       String,
    commit_sha:   Option<String>,
    domain:       Option<String>,
    notify_email: Option<String>,
}

#[derive(Serialize)]
pub struct DeploymentResponse {
    id:         Uuid,
    state:      DeploymentState,
    project:    String,
    branch:     String,
    created_at: String,
    url:        Option<String>,
    error:      Option<String>,
}

#[derive(Deserialize)]
pub struct ChatRequest {
    message: String,
}

#[derive(Serialize)]
pub struct ChatResponse {
    response: String,
}

#[derive(Deserialize)]
pub struct AnalyzeCodeRequest {
    code:     String,
    language: String,
}

pub async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status:  "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

pub async fn create_deployment(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateDeploymentRequest>,
) -> impl IntoResponse {
    let config = DeploymentConfig {
        project_name: request.project_name,
        owner:        request.owner,
        repo:         request.repo,
        branch:       request.branch,
        commit_sha:   request.commit_sha,
        domain:       request.domain,
        notify_email: request.notify_email,
    };

    let deployment = match Deployment::new(config) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code:  400,
                }),
            ).into_response();
        }
    };

    let response = DeploymentResponse {
        id:         deployment.id,
        state:      deployment.state,
        project:    deployment.config.project_name.clone(),
        branch:     deployment.config.branch.clone(),
        created_at: deployment.created_at.to_rfc3339(),
        url:        None,
        error:      None,
    };

    let mut queue = state.queue.write().await;

    if let Err(e) = queue.enqueue(deployment) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
                code:  503,
            }),
        ).into_response();
    }

    (StatusCode::CREATED, Json(response)).into_response()
}

pub async fn get_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let queue = state.queue.read().await;

    let deployment = match queue.find(id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "deployment not found".to_string(),
                    code:  404,
                }),
            ).into_response();
        }
    };

    let response = DeploymentResponse {
        id:         deployment.id,
        state:      deployment.state,
        project:    deployment.config.project_name.clone(),
        branch:     deployment.config.branch.clone(),
        created_at: deployment.created_at.to_rfc3339(),
        url:        deployment.result.as_ref().and_then(|r| r.url.clone()),
        error:      deployment.result.as_ref().and_then(|r| r.error.clone()),
    };

    (StatusCode::OK, Json(response)).into_response()
}

pub async fn cancel_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut queue = state.queue.write().await;

    if let Err(e) = queue.cancel(id) {
        let status_code = e.status_code();
        let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        return (
            status,
            Json(ErrorResponse {
                error: e.to_string(),
                code:  status_code,
            }),
        ).into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({
        "message": "deployment cancelled"
    }))).into_response()
}

pub async fn list_deployments(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let queue = state.queue.read().await;

    let deployments: Vec<DeploymentResponse> = queue
        .iter()
        .map(|d| DeploymentResponse {
            id:         d.id,
            state:      d.state,
            project:    d.config.project_name.clone(),
            branch:     d.config.branch.clone(),
            created_at: d.created_at.to_rfc3339(),
            url:        d.result.as_ref().and_then(|r| r.url.clone()),
            error:      d.result.as_ref().and_then(|r| r.error.clone()),
        })
        .collect();

    (StatusCode::OK, Json(deployments)).into_response()
}

pub async fn chat(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> impl IntoResponse {
    if request.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "message cannot be empty".to_string(),
                code:  400,
            }),
        ).into_response();
    }

    let response = match state.groq.complete(&request.message).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code:  500,
                }),
            ).into_response();
        }
    };

    (StatusCode::OK, Json(ChatResponse { response })).into_response()
}

pub async fn analyze_code(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AnalyzeCodeRequest>,
) -> impl IntoResponse {
    if request.code.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "code cannot be empty".to_string(),
                code:  400,
            }),
        ).into_response();
    }

    if request.language.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "language cannot be empty".to_string(),
                code:  400,
            }),
        ).into_response();
    }

    let analysis = match state.groq.analyze_code(&request.code, &request.language).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code:  500,
                }),
            ).into_response();
        }
    };

    (StatusCode::OK, Json(ChatResponse { response: analysis })).into_response()
}

pub async fn get_repository(
    State(state): State<Arc<AppState>>,
    Path((owner, repo)): Path<(String, String)>,
) -> impl IntoResponse {
    let repository = match state.github.get_repository(&owner, &repo).await {
        Ok(r) => r,
        Err(e) => {
            let status_code = e.status_code();
            let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            return (
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code:  status_code,
                }),
            ).into_response();
        }
    };

    (StatusCode::OK, Json(repository)).into_response()
}

pub async fn list_dns_records(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let records = match state.cloudflare.list_dns_records(None).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code:  502,
                }),
            ).into_response();
        }
    };

    (StatusCode::OK, Json(records)).into_response()
}

pub async fn purge_cache(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if let Err(e) = state.cloudflare.purge_cache(None).await {
        return (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: e.to_string(),
                code:  502,
            }),
        ).into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({
        "message": "cache purged"
    }))).into_response()
}

#[derive(Serialize)]
pub struct HealthReadyResponse {
    status:  String,
    version: String,
    checks:  HealthChecks,
}

#[derive(Serialize)]
pub struct HealthChecks {
    queue:      HealthCheckResult,
    github:     HealthCheckResult,
    s3:         HealthCheckResult,
    cloudflare: HealthCheckResult,
}

#[derive(Serialize)]
pub struct HealthCheckResult {
    status:  String,
    latency: Option<u64>,
    error:   Option<String>,
}

pub async fn health_ready(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let queue_status = {
        let _queue = state.queue.read().await;
        HealthCheckResult {
            status:  "ok".to_string(),
            latency: Some(0),
            error:   None,
        }
    };

    let github_check = async {
        let start = std::time::Instant::now();
        match state.github.get_repository("octocat", "Hello-World").await {
            Ok(_) => HealthCheckResult {
                status:  "ok".to_string(),
                latency: Some(start.elapsed().as_millis() as u64),
                error:   None,
            },
            Err(e) => HealthCheckResult {
                status:  "degraded".to_string(),
                latency: Some(start.elapsed().as_millis() as u64),
                error:   Some(e.to_string()),
            },
        }
    };

    let s3_check = async {
        let start = std::time::Instant::now();
        match state.storage.health_check().await {
            Ok(_) => HealthCheckResult {
                status:  "ok".to_string(),
                latency: Some(start.elapsed().as_millis() as u64),
                error:   None,
            },
            Err(e) => HealthCheckResult {
                status:  "degraded".to_string(),
                latency: Some(start.elapsed().as_millis() as u64),
                error:   Some(e.to_string()),
            },
        }
    };

    let cloudflare_check = async {
        let start = std::time::Instant::now();
        match state.cloudflare.list_dns_records(None).await {
            Ok(_) => HealthCheckResult {
                status:  "ok".to_string(),
                latency: Some(start.elapsed().as_millis() as u64),
                error:   None,
            },
            Err(e) => HealthCheckResult {
                status:  "degraded".to_string(),
                latency: Some(start.elapsed().as_millis() as u64),
                error:   Some(e.to_string()),
            },
        }
    };

    let (github, s3, cloudflare) = tokio::join!(github_check, s3_check, cloudflare_check);

    let overall_status = if github.status == "ok" && s3.status == "ok" && cloudflare.status == "ok" {
        "ok"
    } else {
        "degraded"
    };

    let response = HealthReadyResponse {
        status:  overall_status.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        checks:  HealthChecks {
            queue:      queue_status,
            github,
            s3,
            cloudflare,
        },
    };

    let status_code = if overall_status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status_code, Json(response)).into_response()
}

const WEBHOOK_SIGNATURE_HEADER: &str = "X-Hub-Signature-256";
const WEBHOOK_EVENT_HEADER: &str = "X-GitHub-Event";

#[derive(Deserialize)]
pub struct WebhookPushPayload {
    #[serde(rename = "ref")]
    git_ref:    String,
    repository: WebhookRepository,
    head_commit: Option<WebhookCommit>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct WebhookRepository {
    name:           String,
    full_name:      String,
    default_branch: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct WebhookCommit {
    id:      String,
    message: String,
}

pub async fn github_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let signature = match headers.get(WEBHOOK_SIGNATURE_HEADER) {
        Some(sig) => match sig.to_str() {
            Ok(s) => s,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "invalid signature header".to_string(),
                        code:  400,
                    }),
                ).into_response();
            }
        },
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "missing signature".to_string(),
                    code:  401,
                }),
            ).into_response();
        }
    };

    let webhook_secret = match &state.config.github.webhook_secret {
        Some(secret) => secret,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "webhook secret not configured".to_string(),
                    code:  500,
                }),
            ).into_response();
        }
    };

    match GitHubProvider::verify_webhook_signature(webhook_secret, signature, &body) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "invalid signature".to_string(),
                    code:  401,
                }),
            ).into_response();
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code:  400,
                }),
            ).into_response();
        }
    }

    let event_type = headers
        .get(WEBHOOK_EVENT_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    let event = GitHubProvider::parse_webhook_event(event_type);

    if event != crate::providers::WebhookEvent::Push {
        return (StatusCode::OK, Json(serde_json::json!({
            "message": "event ignored",
            "event": event_type
        }))).into_response();
    }

    let payload: WebhookPushPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("invalid payload: {e}"),
                    code:  400,
                }),
            ).into_response();
        }
    };

    let branch = payload.git_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(&payload.git_ref);

    let parts: Vec<&str> = payload.repository.full_name.split('/').collect();

    if parts.len() != 2 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid repository name".to_string(),
                code:  400,
            }),
        ).into_response();
    }

    let owner = parts[0];
    let repo = parts[1];

    let commit_sha = payload.head_commit.as_ref().map(|c| c.id.clone());

    let config = DeploymentConfig {
        project_name: repo.to_string(),
        owner:        owner.to_string(),
        repo:         repo.to_string(),
        branch:       branch.to_string(),
        commit_sha,
        domain:       None,
        notify_email: None,
    };

    let deployment = match Deployment::new(config) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code:  400,
                }),
            ).into_response();
        }
    };

    let deployment_id = deployment.id;

    let mut queue = state.queue.write().await;

    if let Err(e) = queue.enqueue(deployment) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: e.to_string(),
                code:  503,
            }),
        ).into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({
        "message": "deployment queued",
        "deployment_id": deployment_id
    }))).into_response()
}
