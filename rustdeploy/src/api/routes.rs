use std::sync::Arc;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use super::handlers;
use super::middleware::{
    cors_layer,
    rate_limit,
    request_id,
    require_api_key,
    security_headers,
    RateLimitState,
};
use super::state::AppState;

const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

pub fn create_router(state: Arc<AppState>, rate_limiter: RateLimitState) -> Router {
    let public_routes = Router::new()
        .route("/health", get(handlers::health))
        .route("/health/ready", get(handlers::health_ready))
        .route("/webhook/github", post(handlers::github_webhook));

    let deployment_routes = Router::new()
        .route("/", post(handlers::create_deployment))
        .route("/", get(handlers::list_deployments))
        .route("/{id}", get(handlers::get_deployment))
        .route("/{id}/cancel", post(handlers::cancel_deployment));

    let github_routes = Router::new()
        .route("/repos/{owner}/{repo}", get(handlers::get_repository));

    let cloudflare_routes = Router::new()
        .route("/dns", get(handlers::list_dns_records))
        .route("/cache/purge", post(handlers::purge_cache));

    let ai_routes = Router::new()
        .route("/chat", post(handlers::chat))
        .route("/analyze", post(handlers::analyze_code));

    let protected_routes = Router::new()
        .nest("/deployments", deployment_routes)
        .nest("/github", github_routes)
        .nest("/cloudflare", cloudflare_routes)
        .nest("/ai", ai_routes)
        .layer(middleware::from_fn_with_state(state.clone(), require_api_key));

    Router::new()
        .merge(public_routes)
        .nest("/api/v1", protected_routes)
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(middleware::from_fn_with_state(rate_limiter, rate_limit))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn(cors_layer))
        .layer(middleware::from_fn(request_id))
        .with_state(state)
}
