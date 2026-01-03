use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::RwLock;

use super::state::AppState;

const API_KEY_HEADER: &str = "X-API-Key";
const REQUEST_ID_HEADER: &str = "X-Request-Id";

const RATE_LIMIT_WINDOW_SECONDS: u64 = 60;
const RATE_LIMIT_MAX_REQUESTS: u32 = 100;
const RATE_LIMIT_CLEANUP_THRESHOLD: usize = 10000;

const ALLOWED_ORIGINS: &[&str] = &[];
const ALLOWED_METHODS: &[Method] = &[
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::DELETE,
    Method::OPTIONS,
];

#[derive(Clone)]
pub struct RateLimitState {
    inner: Arc<RwLock<RateLimitInner>>,
}

struct RateLimitInner {
    requests: HashMap<IpAddr, RateLimitEntry>,
    last_cleanup: Instant,
}

struct RateLimitEntry {
    count: u32,
    window_start: Instant,
}

impl RateLimitState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RateLimitInner {
                requests: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
        }
    }

    async fn check_rate_limit(&self, ip: IpAddr) -> bool {
        let mut inner = self.inner.write().await;
        let now = Instant::now();
        let window = Duration::from_secs(RATE_LIMIT_WINDOW_SECONDS);

        if inner.requests.len() > RATE_LIMIT_CLEANUP_THRESHOLD {
            if now.duration_since(inner.last_cleanup) > window {
                inner.requests.retain(|_, entry| {
                    now.duration_since(entry.window_start) < window
                });
                inner.last_cleanup = now;
            }
        }

        let entry = inner.requests.entry(ip).or_insert(RateLimitEntry {
            count: 0,
            window_start: now,
        });

        if now.duration_since(entry.window_start) >= window {
            entry.count = 1;
            entry.window_start = now;
            return true;
        }

        if entry.count >= RATE_LIMIT_MAX_REQUESTS {
            return false;
        }

        entry.count += 1;
        true
    }
}

impl Default for RateLimitState {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn require_api_key(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let api_key = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok());

    let key = match api_key {
        Some(k) if !k.is_empty() => k,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "ApiKey")],
                "missing api key",
            ).into_response();
        }
    };

    if !state.config.api.verify_key(key) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "ApiKey")],
            "invalid api key",
        ).into_response();
    }

    next.run(request).await
}

pub async fn rate_limit(
    State(rate_limiter): State<RateLimitState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let ip = addr.ip();

    if !rate_limiter.check_rate_limit(ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [
                (header::RETRY_AFTER, RATE_LIMIT_WINDOW_SECONDS.to_string()),
            ],
            "rate limit exceeded",
        ).into_response();
    }

    next.run(request).await
}

pub async fn cors_layer(request: Request, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let is_preflight = request.method() == Method::OPTIONS;

    let mut response = if is_preflight {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };

    let headers = response.headers_mut();

    let allowed_origin = match &origin {
        Some(o) if ALLOWED_ORIGINS.is_empty() => o.clone(),
        Some(o) if ALLOWED_ORIGINS.iter().any(|allowed| *allowed == o) => o.clone(),
        _ if ALLOWED_ORIGINS.is_empty() => "*".to_string(),
        _ => return response,
    };

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        allowed_origin.parse().unwrap(),
    );

    if origin.is_some() {
        headers.insert(
            header::VARY,
            "Origin".parse().unwrap(),
        );
    }

    let methods: String = ALLOWED_METHODS
        .iter()
        .map(|m| m.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        methods.parse().unwrap(),
    );

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        "Content-Type, Authorization, X-API-Key".parse().unwrap(),
    );

    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        "86400".parse().unwrap(),
    );

    response
}

pub async fn request_id(request: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();

    let mut response = next.run(request).await;

    response.headers_mut().insert(
        REQUEST_ID_HEADER,
        request_id.parse().unwrap(),
    );

    response
}

pub async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().unwrap(),
    );

    headers.insert(
        header::X_FRAME_OPTIONS,
        "DENY".parse().unwrap(),
    );

    headers.insert(
        "X-XSS-Protection",
        "1; mode=block".parse().unwrap(),
    );

    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().unwrap(),
    );

    response
}
