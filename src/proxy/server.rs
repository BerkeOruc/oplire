use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::config::ProxyConfig;
use crate::proxy::handlers::{handle_messages, handle_models, ProxyState};
use crate::warp::WarpResolver;

pub async fn start_proxy_server(config: ProxyConfig) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let warp_resolver =
        WarpResolver::new(config.max_retries, config.warp_reset_delay_ms);

    let state = Arc::new(Mutex::new(ProxyState {
        config: config.clone(),
        client,
        warp_resolver,
    }));

    let app = Router::new()
        .route("/v1/models", get(handle_models))
        .route("/v1/messages", post(handle_messages))
        .route("/health", get(|| async { "OK" }))
        .with_state(state);

    let addr: SocketAddr = config
        .listen_addr
        .parse()
        .unwrap_or_else(|_| "127.0.0.1:8080".parse().unwrap());

    info!("Starting proxy server on {}", addr);
    info!("OpenCode Zen upstream: {}", config.opencode_base_url);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Proxy server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
