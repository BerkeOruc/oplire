use axum::{
    middleware::{self, Next},
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::config::ProxyConfig;
use crate::proxy::handlers::{handle_messages, handle_model_detail, handle_models, ProxyState};
use crate::warp::WarpResolver;

async fn log_requests(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    info!("→ {} {}", method, uri);
    let response = next.run(request).await;
    info!("← {} {} {}", method, uri, response.status());
    response
}

pub fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(300))
        .build()
}

fn router(config: &ProxyConfig) -> reqwest::Result<Router> {
    let client = build_client()?;

    let warp_resolver = WarpResolver::new(config.max_retries, config.warp_reset_delay_ms);

    let state = Arc::new(Mutex::new(ProxyState {
        config: config.clone(),
        client,
        warp_resolver,
    }));

    Ok(Router::new()
        .route("/v1/models", get(handle_models))
        .route("/v1/models/{model_id}", get(handle_model_detail))
        .route("/v1/messages", post(handle_messages))
        .route("/v1/chat/completions", post(handle_messages))
        .route("/health", get(|| async { "OK" }))
        .layer(middleware::from_fn(log_requests))
        .with_state(state))
}

pub async fn start_proxy_server(config: ProxyConfig) -> anyhow::Result<()> {
    let app = router(&config)?;
    let addr: SocketAddr = config
        .listen_addr
        .parse()
        .unwrap_or_else(|_| "127.0.0.1:8080".parse().unwrap());

    info!("Starting proxy server on {}", addr);
    info!("Inference upstream: {}", config.opencode_base_url);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("Proxy server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Path,
        http::{header::RETRY_AFTER, StatusCode},
        response::Html,
        Json,
    };
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    async fn serve(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    #[tokio::test]
    async fn serves_upstream_model_list_and_detail() {
        let upstream =
            serve(
                Router::new()
                    .route(
                        "/v1/models",
                        get(|| async {
                            Json(json!({"data": [{"id": "demo/model", "name": "Demo"}]}))
                        }),
                    )
                    .route(
                        "/v1/models/{id}",
                        get(|Path(id): Path<String>| async move {
                            Json(json!({"id": id, "name": "Demo"}))
                        }),
                    ),
            )
            .await;
        let proxy = serve(
            router(&ProxyConfig {
                opencode_base_url: upstream,
                ..ProxyConfig::default()
            })
            .unwrap(),
        )
        .await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let list: Value = client
            .get(format!("{proxy}/v1/models"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(list["data"][0]["display_name"], "Demo");
        assert_eq!(list["first_id"], "demo/model");
        assert!(list["data"][0].get("input_price").is_none());

        let detail: Value = client
            .get(format!("{proxy}/v1/models/demo%2Fmodel"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(detail["id"], "demo/model");
    }

    #[tokio::test]
    async fn model_listing_reports_html_and_rate_limit_responses() {
        let upstream = serve(
            Router::new()
                .route(
                    "/html/v1/models",
                    get(|| async { Html("<html>OpenCode</html>") }),
                )
                .route(
                    "/limit/v1/models",
                    get(|| async { (StatusCode::TOO_MANY_REQUESTS, [(RETRY_AFTER, "60")]) }),
                ),
        )
        .await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        for (path, status) in [
            ("html", StatusCode::BAD_GATEWAY),
            ("limit", StatusCode::TOO_MANY_REQUESTS),
        ] {
            let proxy = serve(
                router(&ProxyConfig {
                    opencode_base_url: format!("{upstream}/{path}"),
                    ..ProxyConfig::default()
                })
                .unwrap(),
            )
            .await;
            let response = client
                .get(format!("{proxy}/v1/models"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            if status == StatusCode::TOO_MANY_REQUESTS {
                assert_eq!(response.headers()[RETRY_AFTER], "60");
            }
        }
    }
}
