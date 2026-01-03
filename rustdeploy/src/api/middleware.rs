use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

const API_KEY_HEADER: &str = "X-API-Key";
const API_KEY_LENGTH_MIN: usize = 32;

pub async fn require_api_key(request: Request, next: Next) -> Response {
    let api_key = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|v| v.to_str().ok());

    let key = match api_key {
        Some(k) => k,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                "missing api key",
            ).into_response();
        }
    };

    if key.len() < API_KEY_LENGTH_MIN {
        return (
            StatusCode::UNAUTHORIZED,
            "invalid api key",
        ).into_response();
    }

    next.run(request).await
}

pub async fn cors_layer(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;

    let headers = response.headers_mut();

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        "*".parse().unwrap(),
    );

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        "GET, POST, PUT, DELETE, OPTIONS".parse().unwrap(),
    );

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        "Content-Type, Authorization, X-API-Key".parse().unwrap(),
    );

    response
}

pub async fn request_id(request: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();

    let mut response = next.run(request).await;

    response.headers_mut().insert(
        "X-Request-Id",
        request_id.parse().unwrap(),
    );

    response
}
