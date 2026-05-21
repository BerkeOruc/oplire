use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::config::ProxyConfig;
use crate::transform::{anthropic_to_opencode_request, opencode_stream_to_anthropic, opencode_response_to_anthropic};
use crate::warp::WarpResolver;

pub struct ProxyState {
    pub config: ProxyConfig,
    pub client: Client,
    pub warp_resolver: WarpResolver,
}

pub async fn handle_models(State(_state): State<Arc<Mutex<ProxyState>>>) -> impl IntoResponse {
    let response = ProxyConfig::models_response();
    (StatusCode::OK, Json(response))
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
    drop(state_guard);

    let mut retry_count = 0;

    loop {
        let result = if is_stream {
            forward_streaming(&base_url, &api_key, &opencode_body, &model).await
        } else {
            forward_non_streaming(&base_url, &api_key, &opencode_body, &model).await
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
) -> Result<Response, ProxyError> {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let mut request = reqwest::Client::new()
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
) -> Result<Response, ProxyError> {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let mut request = reqwest::Client::new()
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
