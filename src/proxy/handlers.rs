use axum::{
    body::Body,
    extract::State,
    http::{header::RETRY_AFTER, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::config::ProxyConfig;
use crate::proxy::server::build_client;
use crate::transform::{anthropic_to_opencode_request, opencode_stream_to_anthropic, opencode_response_to_anthropic};
use crate::warp::WarpResolver;

pub struct ProxyState {
    pub config: ProxyConfig,
    pub client: Client,
    pub warp_resolver: WarpResolver,
}

pub async fn handle_models(State(state): State<Arc<Mutex<ProxyState>>>) -> Response {
    let state_guard = state.lock().await;
    let base_url = state_guard.config.opencode_base_url.clone();
    let api_key = state_guard.config.opencode_api_key.clone();
    let client = state_guard.client.clone();
    drop(state_guard);

    let models_url = format!("{}/v1/models", base_url.trim_end_matches('/'));

    let mut request = client
        .get(&models_url)
        .header("Accept", "application/json");

    if let Some(key) = &api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    match request.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(body) => match body.get("data").and_then(Value::as_array) {
                Some(models) => Json(transform_models_to_anthropic(models)).into_response(),
                None => (StatusCode::BAD_GATEWAY, "Upstream response has no model list")
                    .into_response(),
            },
            Err(e) => {
                warn!("Failed to parse models response: {}", e);
                (StatusCode::BAD_GATEWAY, "Invalid upstream models response").into_response()
            }
        },
        Ok(resp) => {
            let mut response = (resp.status(), "Upstream model listing failed").into_response();
            if let Some(retry_after) = resp.headers().get(RETRY_AFTER) {
                response.headers_mut().insert(RETRY_AFTER, retry_after.clone());
            }
            response
        }
        Err(e) => {
            warn!("Failed to fetch models from upstream: {}", e);
            (StatusCode::BAD_GATEWAY, "Could not reach upstream model endpoint").into_response()
        }
    }
}

fn transform_models_to_anthropic(upstream: &[Value]) -> Value {
    let models: Vec<Value> = upstream
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            if id.is_empty() {
                return None;
            }

            let display_name = match m.get("name").and_then(Value::as_str) {
                Some(name) => name.to_string(),
                None => id
                    .replace('-', " ")
                    .split_whitespace()
                    .map(|w| {
                        let mut chars = w.chars();
                        match chars.next() {
                            None => String::new(),
                            Some(c) => {
                                let upper = c.to_uppercase().collect::<String>();
                                format!("{}{}", upper, chars.as_str())
                            }
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            };

            let mut model = serde_json::Map::new();
            model.insert("id".into(), Value::String(id.into()));
            model.insert("name".into(), Value::String(display_name.clone()));
            model.insert("type".into(), Value::String("model".into()));
            model.insert("display_name".into(), Value::String(display_name));
            let context_window = m
                .get("context_window")
                .and_then(Value::as_u64)
                .or_else(|| m.get("max_tokens").and_then(Value::as_u64));
            if let Some(window) = context_window {
                model.insert("context_window".into(), Value::from(window));
            }
            for key in [
                "created", "owned_by", "architecture", "input_price", "output_price",
            ] {
                if let Some(value) = m.get(key) {
                    model.insert(key.into(), value.clone());
                }
            }

            Some(Value::Object(model))
        })
        .collect();

    serde_json::json!({
        "data": models,
        "has_more": false,
        "first_id": models.first().and_then(|m| m.get("id").and_then(|v| v.as_str())).unwrap_or(""),
        "last_id": models.last().and_then(|m| m.get("id").and_then(|v| v.as_str())).unwrap_or("")
    })
}

pub async fn handle_model_detail(
    State(state): State<Arc<Mutex<ProxyState>>>,
    axum::extract::Path(model_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let state_guard = state.lock().await;
    let base_url = state_guard.config.opencode_base_url.clone();
    let api_key = state_guard.config.opencode_api_key.clone();
    let client = state_guard.client.clone();
    drop(state_guard);

    if matches!(model_id.as_str(), "" | "." | "..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(mut model_url) = url::Url::parse(&base_url) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    model_url.set_query(None);
    model_url.set_fragment(None);
    match model_url.path_segments_mut() {
        Ok(mut path) => {
            path.pop_if_empty().extend(["v1", "models", &model_id]);
        }
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    }

    let mut request = client
        .get(model_url)
        .header("Accept", "application/json");

    if let Some(key) = &api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    match request.send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<Value>().await {
                Ok(model) => {
                    let mut transformed = serde_json::Map::new();
                    if let Some(id) = model.get("id").and_then(|v| v.as_str()) {
                        transformed.insert("id".to_string(), Value::String(id.to_string()));
                    }
                    if let Some(name) = model.get("name").and_then(|v| v.as_str()) {
                        transformed.insert("name".to_string(), Value::String(name.to_string()));
                        transformed.insert("display_name".to_string(), Value::String(name.to_string()));
                    }
                    transformed.insert("type".to_string(), Value::String("model".to_string()));

                    (StatusCode::OK, Json(Value::Object(transformed))).into_response()
                }
                Err(e) => {
                    error!("Failed to parse model detail: {}", e);
                    StatusCode::NOT_FOUND.into_response()
                }
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn handle_messages(
    State(state): State<Arc<Mutex<ProxyState>>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request_body = match serde_json::from_slice::<Value>(&body) {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to parse request body: {}", e);
            return error_response(&format!("Invalid JSON: {}", e));
        }
    };

    let is_stream = request_body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let model = request_body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let opencode_body = anthropic_to_opencode_request(&request_body);

    let state_guard = state.lock().await;
    let base_url = state_guard.config.opencode_base_url.clone();
    let api_key = state_guard.config.opencode_api_key.clone();
    let max_retries = state_guard.config.max_retries;
    let reset_delay = state_guard.config.warp_reset_delay_ms;
    let mut client = state_guard.client.clone();
    drop(state_guard);

    let mut retry_count = 0;

    loop {
        let result = if is_stream {
            forward_streaming(&base_url, &api_key, &opencode_body, &model, &client).await
        } else {
            forward_non_streaming(&base_url, &api_key, &opencode_body, &model, &client).await
        };

        match result {
            Ok(response) => return response,
            Err(ProxyError::RateLimited) => {
                retry_count += 1;
                if retry_count > max_retries {
                    return error_response("Rate limit exceeded after WARP resets");
                }

                info!(
                    "429 received, attempting WARP reset (attempt {}/{})",
                    retry_count, max_retries
                );

                let resolver = WarpResolver::new(max_retries, reset_delay);
                if !resolver.handle_429(retry_count - 1).await {
                    return error_response("WARP reset failed, rate limit still active");
                }
                if let Ok(fresh) = build_client() {
                    state.lock().await.client = fresh.clone();
                    client = fresh;
                    info!("Rebuilt HTTP client pool after WARP reset");
                }
            }
            Err(ProxyError::RequestFailed(msg)) => {
                error!("Upstream request failed: {}", msg);
                return error_response(&format!("Upstream error: {}", msg));
            }
        }
    }
}

async fn forward_streaming(
    base_url: &str,
    api_key: &Option<String>,
    body: &Value,
    model: &str,
    client: &Client,
) -> Result<Response, ProxyError> {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let mut request = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(body);

    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request
        .send()
        .await
        .map_err(|e| ProxyError::RequestFailed(e.to_string()))?;

    if response.status() == 429 {
        return Err(ProxyError::RateLimited);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(ProxyError::RequestFailed(format!(
            "Upstream {} : {}",
            status, body
        )));
    }

    let model_owned = model.to_string();
    let stream = response
        .bytes_stream()
        .map(move |chunk| {
            let chunk = chunk.map_err(std::io::Error::other)?;
            let lines = String::from_utf8_lossy(&chunk);
            let mut output = String::new();

            for line in lines.lines() {
                if let Some(transformed) = opencode_stream_to_anthropic(line, &model_owned) {
                    output.push_str(&transformed);
                }
            }

            if output.is_empty() {
                Ok::<Bytes, std::io::Error>(Bytes::from(lines.into_owned()))
            } else {
                Ok::<Bytes, std::io::Error>(Bytes::from(output))
            }
        });

    let body = Body::from_stream(stream);

    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        "Content-Type",
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response.headers_mut().insert(
        "Cache-Control",
        HeaderValue::from_static("no-cache"),
    );
    response
        .headers_mut()
        .insert("Connection", HeaderValue::from_static("keep-alive"));
    response
        .headers_mut()
        .insert("X-Accel-Buffering", HeaderValue::from_static("no"));

    Ok(response)
}

async fn forward_non_streaming(
    base_url: &str,
    api_key: &Option<String>,
    body: &Value,
    model: &str,
    client: &Client,
) -> Result<Response, ProxyError> {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let mut request = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(body);

    if let Some(key) = api_key {
        request = request.header("Authorization", format!("Bearer {}", key));
    }

    let response = request
        .send()
        .await
        .map_err(|e| ProxyError::RequestFailed(e.to_string()))?;

    if response.status() == 429 {
        return Err(ProxyError::RateLimited);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(ProxyError::RequestFailed(format!(
            "Upstream {} : {}",
            status, body
        )));
    }

    let opencode_response = response
        .json::<Value>()
        .await
        .map_err(|e| ProxyError::RequestFailed(e.to_string()))?;

    let anthropic_response = opencode_response_to_anthropic(&opencode_response, model);

    let mut headers = HeaderMap::new();
    headers.insert(
        "Content-Type",
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "x-request-id",
        HeaderValue::from_str(&format!("req_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))).unwrap_or(HeaderValue::from_static("unknown")),
    );

    Ok((StatusCode::OK, headers, Json(anthropic_response)).into_response())
}

fn error_response(message: &str) -> Response {
    let body = serde_json::json!({
        "error": {
            "message": message,
            "type": "api_error",
            "code": "internal_error"
        }
    });
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(body),
    )
        .into_response()
}

#[derive(Debug)]
enum ProxyError {
    RateLimited,
    RequestFailed(String),
}
