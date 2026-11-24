use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware::{self, Next},
    routing::{get, post},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use url::{Host, Url};

use crate::config::ProxyConfig;
use crate::proxy::handlers::{
    ProxyState, error_response, handle_messages, handle_model_detail, handle_models,
};

async fn local_requests(
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> axum::response::Response {
    use axum::http::{StatusCode, header, uri::Authority};

    if request.headers().contains_key(header::ORIGIN) {
        return error_response(
            StatusCode::FORBIDDEN,
            "Browser-origin requests are not supported",
        );
    }
    let host = match request.headers().get(header::HOST) {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|host| host.parse::<Authority>().ok()),
        None => request.uri().authority().cloned(),
    };
    if !host.as_ref().is_some_and(|authority| {
        let host = authority.host();
        host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }) {
        return error_response(
            StatusCode::FORBIDDEN,
            "The bridge only accepts requests to a loopback host",
        );
    }
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
        .merge(
            Router::new()
                .route("/v1/messages", post(handle_messages))
                .route("/v1/chat/completions", post(handle_messages))
                .layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route("/health", get(|| async { "OK" }))
        .layer(middleware::from_fn(local_requests))
        .with_state(state))
}

pub async fn start_proxy_server(config: ProxyConfig) -> anyhow::Result<()> {
    let (listener, app) = bind_proxy(&config).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn bind_proxy(config: &ProxyConfig) -> anyhow::Result<(tokio::net::TcpListener, Router)> {
    let addr: SocketAddr = config.listen_addr.parse()?;
    anyhow::ensure!(addr.ip().is_loopback(), "listen address must be loopback");
    let app = router(config)?;

    info!("Starting proxy server on {}", addr);
    info!("Inference upstream: {}", config.opencode_base_url);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(
        "Proxy server listening on http://{}",
        listener.local_addr()?
    );
    Ok((listener, app))
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
    use bytes::Bytes;
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
            post(|Json(body): Json<Value>| async move {
                let error = if body["model"] == "demo/context" {
                    json!({"code": "context_length_exceeded", "message": "Maximum context is 2048 tokens"})
                } else {
                    json!({"message": "Unknown model: demo/model"})
                };
                (StatusCode::BAD_REQUEST, Json(json!({"error": error})))
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
        for (model, message) in [
            ("demo/model", "Unknown model: demo/model"),
            (
                "demo/context",
                "Prompt is too long: Maximum context is 2048 tokens",
            ),
        ] {
            let response = client
                .post(format!("{proxy}/v1/messages"))
                .json(&json!({"model": model, "messages": []}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["error"]["type"], "invalid_request_error");
            assert_eq!(body["error"]["message"], message);
        }
    }

    #[tokio::test]
    async fn inference_translates_streams_and_rejects_non_stream_responses() {
        use axum::http::header::CONTENT_TYPE;

        let upstream = serve(Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                use axum::response::IntoResponse;
                assert_eq!(body["stream_options"], json!({"include_usage": true}));
                match body["model"].as_str() {
                    Some("demo/stream") => (
                        [(CONTENT_TYPE, "text/event-stream; charset=utf-8")],
                        concat!(
                            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":\"stop\"}]}\n\n",
                            "data: [DONE]\n\n"
                        ),
                    ).into_response(),
                    _ => Html("<html>not an inference server</html>").into_response(),
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
        for (model, status) in [
            ("demo/stream", StatusCode::OK),
            ("demo/html", StatusCode::BAD_GATEWAY),
        ] {
            let response = client
                .post(format!("{proxy}/v1/messages"))
                .json(&json!({"model": model, "messages": [], "stream": true}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            if status == StatusCode::OK {
                assert_eq!(
                    response.headers()[CONTENT_TYPE],
                    "text/event-stream; charset=utf-8"
                );
                let output = response.text().await.unwrap();
                assert!(output.starts_with("event: message_start\n"));
                assert!(output.contains("\"text\":\"Hello\""));
                assert!(
                    output.ends_with("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n")
                );
            }
        }
    }

    #[tokio::test]
    async fn inference_round_trips_tools_without_forwarding_client_credentials() {
        let upstream = serve(Router::new().route(
            "/v1/chat/completions",
            post(|headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer upstream-fixture");
                assert!(!headers.contains_key("x-api-key"));
                assert!(!headers.contains_key("anthropic-beta"));
                assert_eq!(body["tools"][0], json!({"type":"function","function":{
                    "name":"lookup","description":"Find a label","parameters":{"type":"object"}
                }}));
                let messages = body["messages"].as_array().unwrap();
                if messages.len() == 1 {
                    assert_eq!(body["tool_choice"], json!({"type":"function","function":{"name":"lookup"}}));
                    assert_eq!(body["parallel_tool_calls"], false);
                    Json(json!({"choices":[{"message":{"content":"Checking.","tool_calls":[{
                        "id":"call_label","type":"function","function":{"name":"lookup","arguments":"{\"key\":\"label\"}"}
                    }]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":13,"completion_tokens":6}}))
                } else {
                    assert_eq!(messages[1]["tool_calls"][0]["id"], "call_label");
                    assert_eq!(messages[1]["tool_calls"][0]["function"]["arguments"], "{\"key\":\"label\"}");
                    assert_eq!(messages[2], json!({"role":"tool","tool_call_id":"call_label","content":"blue"}));
                    assert_eq!(messages[3], json!({"role":"user","content":"Answer briefly."}));
                    assert_eq!(messages[4], json!({"role":"system","content":"A reminder"}));
                    assert_eq!(messages.len(), 5);
                    Json(json!({"choices":[{"message":{"content":"blue"},"finish_reason":"stop"}]}))
                }
            }),
        )).await;
        let proxy = serve(
            router(&ProxyConfig {
                opencode_base_url: upstream,
                opencode_api_key: Some("upstream-fixture".into()),
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
        let send = |body: Value| {
            client
                .post(format!("{proxy}/v1/messages?beta=true"))
                .bearer_auth("client-fixture")
                .header("x-api-key", "client-fixture")
                .header("anthropic-beta", "client-fixture")
                .json(&body)
                .send()
        };
        let mut body = json!({
            "model":"demo/model", "messages":[{"role":"user","content":"Find a label."}],
            "tools":[{"name":"lookup","description":"Find a label","input_schema":{"type":"object"}}],
            "tool_choice":{"type":"tool","name":"lookup","disable_parallel_tool_use":true}
        });
        let response: Value = send(body.clone())
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            response["usage"],
            json!({"input_tokens":13,"output_tokens":6})
        );
        assert_eq!(response["stop_reason"], "tool_use");
        assert_eq!(
            response["content"][1],
            json!({"type":"tool_use","id":"call_label","name":"lookup","input":{"key":"label"}})
        );
        body["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":response["content"]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"call_label","content":[{"type":"text","text":"blue"}]},{"type":"text","text":"Answer briefly."}]}),
            json!({"role":"system","content":[{"type":"text","text":"A reminder"}]}),
            json!({"role":"system","content":[],"output_config":{"effort":"low"}}),
        ]);
        body.as_object_mut().unwrap().remove("tool_choice");
        let response: Value = send(body)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(response["content"], json!([{"type":"text","text":"blue"}]));
        assert_eq!(response["stop_reason"], "end_turn");
    }

    #[tokio::test]
    async fn inference_enforces_body_size_with_an_anthropic_error() {
        let upstream = serve(Router::new().route("/v1/chat/completions", post(|body: Bytes| async move {
            assert!(body.len() > 2 * 1024 * 1024);
            Json(json!({"choices":[{"message":{"content":"received"},"finish_reason":"stop"}]}))
        })).layer(DefaultBodyLimit::max(4 * 1024 * 1024))).await;
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
        let large = client.post(format!("{proxy}/v1/messages"))
            .json(&json!({"model":"fixture","messages":[{"role":"user","content":"x".repeat(3 * 1024 * 1024)}]}))
            .send().await.unwrap();
        assert_eq!(large.status(), StatusCode::OK);
        let too_large = client
            .post(format!("{proxy}/v1/messages"))
            .body(vec![b'x'; 32 * 1024 * 1024 + 1])
            .send()
            .await
            .unwrap();
        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let error = too_large.json::<Value>().await.unwrap();
        assert_eq!(error["error"]["type"], "request_too_large");
    }

    #[tokio::test]
    async fn rejects_nonlocal_listeners_and_browser_requests_before_inference() {
        let config = ProxyConfig {
            listen_addr: "0.0.0.0:0".into(),
            ..ProxyConfig::default()
        };
        let error = bind_proxy(&config).await.unwrap_err();
        assert_eq!(error.to_string(), "listen address must be loopback");

        let upstream = serve(Router::new().route(
            "/v1/chat/completions",
            post(|| async { StatusCode::IM_A_TEAPOT }),
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
        for (header, value, status) in [
            ("origin", "https://example.com", StatusCode::FORBIDDEN),
            ("host", "example.com", StatusCode::FORBIDDEN),
            (
                "content-type",
                "text/plain",
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ),
        ] {
            let mut request = client
                .post(format!("{proxy}/v1/messages"))
                .body(r#"{"model":"fixture","messages":[]}"#)
                .header(header, value);
            if header != "content-type" {
                request = request.header("content-type", "application/json");
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), status);
        }
    }
}
