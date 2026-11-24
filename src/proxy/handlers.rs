use axum::{
    Json,
    body::Body,
    extract::{State, rejection::BytesRejection},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_TYPE, RETRY_AFTER},
    },
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures_util::stream;
use reqwest::Client;
use serde_json::Value;
use std::{convert::Infallible, sync::Arc};
use tracing::warn;

use crate::config::ProxyConfig;
use crate::transform::{
    anthropic_to_opencode_request, opencode_response_to_anthropic, stream::MessageStream,
};

pub struct ProxyState {
    pub config: ProxyConfig,
    pub client: Client,
}

pub async fn handle_models(State(state): State<Arc<ProxyState>>) -> Response {
    let models_url = format!(
        "{}/v1/models",
        state.config.opencode_base_url.trim_end_matches('/')
    );

    let mut request = state
        .client
        .get(&models_url)
        .header("Accept", "application/json");

    if let Some(key) = &state.config.opencode_api_key {
        request = request.bearer_auth(key);
    }

    match request.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(body) => match body.get("data").and_then(Value::as_array) {
                Some(models) => Json(transform_models_to_anthropic(models)).into_response(),
                None => (
                    StatusCode::BAD_GATEWAY,
                    "Upstream response has no model list",
                )
                    .into_response(),
            },
            Err(e) => {
                warn!("Failed to parse models response: {}", e);
                (StatusCode::BAD_GATEWAY, "Invalid upstream models response").into_response()
            }
        },
        Ok(resp) => upstream_error(&resp, "Upstream model listing failed"),
        Err(e) => upstream_transport_error(e),
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
                "created",
                "owned_by",
                "architecture",
                "input_price",
                "output_price",
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
    State(state): State<Arc<ProxyState>>,
    axum::extract::Path(model_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    if matches!(model_id.as_str(), "" | "." | "..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(mut model_url) = url::Url::parse(&state.config.opencode_base_url) else {
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

    let mut request = state
        .client
        .get(model_url)
        .header("Accept", "application/json");

    if let Some(key) = &state.config.opencode_api_key {
        request = request.bearer_auth(key);
    }

    match request.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
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
                warn!("Failed to parse model detail: {}", e);
                StatusCode::BAD_GATEWAY.into_response()
            }
        },
        Ok(resp) => upstream_error(&resp, "Upstream model lookup failed"),
        Err(e) => upstream_transport_error(e),
    }
}

pub async fn handle_messages(
    State(state): State<Arc<ProxyState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(error) if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return error_response(
                error.status(),
                "The bridge accepts request bodies up to 32 MiB",
            );
        }
        Err(error) => return error_response(error.status(), "Could not read the request body"),
    };
    if !has_media_type(&headers, "application/json") {
        return error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Expected application/json",
        );
    }
    let request_body = match serde_json::from_slice::<Value>(&body) {
        Ok(v) => v,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {e}"));
        }
    };

    let upstream_body = match anthropic_to_opencode_request(&request_body) {
        Ok(body) => body,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };
    let is_stream = upstream_body["stream"] == true;
    let model = upstream_body["model"].as_str().expect("validated model ID");
    let url = format!(
        "{}/v1/chat/completions",
        state.config.opencode_base_url.trim_end_matches('/')
    );
    let mut request = state.client.post(&url).json(&upstream_body);
    if is_stream {
        request = request.header("Accept", "text/event-stream");
    }
    if let Some(key) = &state.config.opencode_api_key {
        request = request.bearer_auth(key);
    }
    match request.send().await {
        Ok(resp) if !resp.status().is_success() => {
            let mut resp = resp;
            let detail = match resp.status() {
                StatusCode::BAD_REQUEST
                | StatusCode::NOT_FOUND
                | StatusCode::UNPROCESSABLE_ENTITY => upstream_validation_message(&mut resp).await,
                _ => None,
            };
            upstream_error(
                &resp,
                detail
                    .as_deref()
                    .unwrap_or("Upstream inference request failed"),
            )
        }
        Ok(resp) if is_stream => forward_streaming(resp, model),
        Ok(resp) => match resp.json::<Value>().await {
            Ok(body) => {
                let body = match opencode_response_to_anthropic(&body, model) {
                    Ok(body) => body,
                    Err(message) => return error_response(StatusCode::BAD_GATEWAY, message),
                };
                let mut response = Json(body).into_response();
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_millis());
                if let Ok(id) = HeaderValue::from_str(&format!("req_{millis}")) {
                    response.headers_mut().insert("x-request-id", id);
                }
                response
            }
            Err(e) => upstream_transport_error(e),
        },
        Err(e) => upstream_transport_error(e),
    }
}

async fn upstream_validation_message(upstream: &mut reqwest::Response) -> Option<String> {
    const MAX_ERROR_BYTES: usize = 8 * 1024;
    let mut body = Vec::new();
    while let Some(chunk) = upstream.chunk().await.ok()? {
        if chunk.len() > MAX_ERROR_BYTES - body.len() {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let error: Value = serde_json::from_slice(&body).ok()?;
    let message = error.get("error")?.get("message")?.as_str()?.trim();
    if error["error"]["code"] == "context_length_exceeded" {
        return Some(format!("Prompt is too long: {message}"));
    }
    (!message.is_empty()).then(|| message.to_owned())
}

fn has_media_type(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
}

fn forward_streaming(response: reqwest::Response, model: &str) -> Response {
    if !has_media_type(response.headers(), "text/event-stream") {
        return error_response(
            StatusCode::BAD_GATEWAY,
            "Upstream did not return an event stream",
        );
    }

    let stream = stream::unfold(
        (response, MessageStream::new(model)),
        |(mut upstream, mut message)| async move {
            while !message.is_finished() {
                let output = match upstream.chunk().await {
                    Ok(Some(chunk)) => message.push(&chunk),
                    Ok(None) => message.eof(),
                    Err(error) => {
                        warn!("Upstream event stream failed: {}", error.without_url());
                        message.fail("Upstream event stream was interrupted")
                    }
                };
                if !output.is_empty() {
                    return Some((
                        Ok::<_, Infallible>(Bytes::from(output)),
                        (upstream, message),
                    ));
                }
            }
            None
        },
    );
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        "Content-Type",
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert("Cache-Control", HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert("X-Accel-Buffering", HeaderValue::from_static("no"));

    response
}

pub(super) fn error_response(status: StatusCode, message: &str) -> Response {
    let error_type = match status {
        StatusCode::BAD_REQUEST
        | StatusCode::UNPROCESSABLE_ENTITY
        | StatusCode::UNSUPPORTED_MEDIA_TYPE => "invalid_request_error",
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::FORBIDDEN => "permission_error",
        StatusCode::NOT_FOUND => "not_found_error",
        StatusCode::PAYLOAD_TOO_LARGE => "request_too_large",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        _ => "api_error",
    };
    let body = serde_json::json!({
        "type": "error",
        "error": {
            "message": message,
            "type": error_type,
        }
    });
    (status, Json(body)).into_response()
}

fn upstream_error(upstream: &reqwest::Response, message: &str) -> Response {
    let status = upstream.status();
    if !status.is_client_error() && !status.is_server_error() {
        return error_response(StatusCode::BAD_GATEWAY, "Unexpected upstream status");
    }
    let mut response = error_response(status, message);
    for name in [
        RETRY_AFTER,
        axum::http::HeaderName::from_static("x-should-retry"),
    ] {
        if let Some(value) = upstream.headers().get(&name) {
            response.headers_mut().insert(name, value.clone());
        }
    }
    response
}

fn upstream_transport_error(error: reqwest::Error) -> Response {
    let timed_out = error.is_timeout();
    warn!("Upstream request failed: {}", error.without_url());
    if timed_out {
        error_response(StatusCode::GATEWAY_TIMEOUT, "Upstream request timed out")
    } else {
        error_response(StatusCode::BAD_GATEWAY, "Upstream request failed")
    }
}
