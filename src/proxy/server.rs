use axum::{
    Router,
    middleware::{self, Next},
    routing::{get, post},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use url::{Host, Url};

use crate::config::ProxyConfig;
use crate::proxy::handlers::{ProxyState, handle_messages, handle_model_detail, handle_models};

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

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.trim_end_matches('.').eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn build_client(upstream: &str) -> reqwest::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(300));
    if Url::parse(upstream).is_ok_and(|url| is_loopback(&url)) {
        builder = builder.no_proxy();
    }
    builder.build()
}

fn router(config: &ProxyConfig) -> reqwest::Result<Router> {
    let client = build_client(&config.opencode_base_url)?;

    let state = Arc::new(ProxyState {
        config: config.clone(),
        client,
    });

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
        Json,
        extract::Path,
        http::{StatusCode, header::RETRY_AFTER},
        response::Html,
    };
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    #[test]
    fn only_loopback_upstreams_bypass_the_system_proxy() {
        for host in ["localhost", "localhost.", "127.0.0.1", "[::1]"] {
            assert!(is_loopback(
                &Url::parse(&format!("http://{host}:3000")).unwrap()
            ));
        }
        for host in ["localhost.example.com", "192.168.0.1", "[::2]"] {
            assert!(!is_loopback(
                &Url::parse(&format!("http://{host}:3000")).unwrap()
            ));
        }
    }

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

    #[tokio::test]
    async fn inference_rate_limits_are_returned_without_retries() {
        const RETRY_DATE: &str = "Tue, 24 Nov 2026 12:00:00 GMT";
        let calls = Arc::new(AtomicUsize::new(0));
        let upstream = serve(Router::new().route(
            "/v1/chat/completions",
            post({
                let calls = Arc::clone(&calls);
                move |Json(body): Json<Value>| async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let retry_after = if body["stream"] == true {
                        RETRY_DATE
                    } else {
                        "60"
                    };
                    (StatusCode::TOO_MANY_REQUESTS, [(RETRY_AFTER, retry_after)])
                }
            }),
        ))
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

        for (index, stream) in [false, true].into_iter().enumerate() {
            let response = client
                .post(format!("{proxy}/v1/messages"))
                .json(&json!({
                    "model": "demo/model",
                    "max_tokens": 16,
                    "messages": [{"role": "user", "content": "ping"}],
                    "stream": stream,
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(
                response.headers()[RETRY_AFTER],
                if stream { RETRY_DATE } else { "60" }
            );
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["type"], "error");
            assert_eq!(body["error"]["type"], "rate_limit_error");
            assert_eq!(calls.load(Ordering::Relaxed), index + 1);
        }
    }

    #[tokio::test]
    async fn inference_preserves_structured_provider_validation_messages() {
        let upstream = serve(Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"message": "Unknown model: demo/model"}})),
                )
            }),
        ))
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
        let response = client
            .post(format!("{proxy}/v1/messages"))
            .json(&json!({"model": "demo/model", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["message"], "Unknown model: demo/model");
    }
}
