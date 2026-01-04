use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use rustdeploy::api::{create_router, AppState, RateLimitState};
use rustdeploy::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    info!("Loading configuration...");

    let config = Config::from_env()?;

    let host = config.server.host.clone();
    let port = config.server.port;

    info!("Initializing application state...");

    let state = AppState::new(config).await?;
    let rate_limiter = RateLimitState::new();

    let router = create_router(state, rate_limiter)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{host}:{port}").parse()?;

    info!("Starting server on {addr}");

    let listener = TcpListener::bind(addr).await?;

    axum::serve(listener, router).await?;

    Ok(())
}
