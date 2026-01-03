use std::sync::Arc;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::deployment::{Deployment, DeploymentConfig, DeploymentState};

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
