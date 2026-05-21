use serde_json::{json, Value};

pub fn anthropic_to_opencode_request(body: &Value) -> Value {
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let max_tokens = body.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(4096);
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let temperature = body.get("temperature").cloned();
    let top_p = body.get("top_p").cloned();

    let mut messages: Vec<Value> = Vec::new();

    if let Some(system) = body.get("system") {
        let system_content = match system {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if !system_content.is_empty() {
            messages.push(json!({
                "role": "system",
                "content": system_content
            }));
        }
    }

    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = match msg.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Some(Value::Null) | None => String::new(),
                _ => String::new(),
            };
            let openai_role = match role {
                "assistant" => "assistant",
                _ => "user",
            };
            messages.push(json!({
                "role": openai_role,
                "content": content
            }));
        }
    }

    let mut req = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
        "max_tokens": max_tokens
    });

    if let Some(obj) = req.as_object_mut() {
        if let Some(t) = temperature {
            obj.insert("temperature".to_string(), t);
        }
        if let Some(p) = top_p {
            obj.insert("top_p".to_string(), p);
        }
    }

    req
}

pub fn opencode_response_to_anthropic(body: &Value, model: &str) -> Value {
    let mut text = String::new();
    let mut finish_reason = "stop";

    if let Some(choices) = body.get("choices").and_then(|v| v.as_array()) {
        if let Some(choice) = choices.first() {
            if let Some(message) = choice.get("message") {
                if let Some(content) = message.get("content").and_then(|v| v.as_str()) {
                    text = content.to_string();
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                finish_reason = match reason {
                    "stop" => "end_turn",
                    "length" => "max_tokens",
                    _ => "end_turn",
                };
            }
        }
    }

    let usage = body.get("usage").cloned().unwrap_or(json!({
        "input_tokens": 0,
        "output_tokens": 0
    }));

    json!({
        "id": format!("msg_{}", uuid_short()),
        "type": "message",
        "role": "assistant",
        "content": [{
            "type": "text",
            "text": text
        }],
        "model": model,
        "stop_reason": finish_reason,
        "stop_sequence": null,
        "usage": usage
    })
}

pub fn opencode_stream_to_anthropic(line: &str, _model: &str) -> Option<String> {
    if line.is_empty() || line.starts_with(':') {
        return None;
    }

    if let Some(data) = line.strip_prefix("data: ") {
        if data == "[DONE]" {
            return Some(
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string(),
            );
        }

        if let Ok(val) = serde_json::from_str::<Value>(data) {
            if let Some(choices) = val.get("choices").and_then(|v| v.as_array()) {
                if let Some(choice) = choices.first() {
                    let delta_text = choice
                        .get("delta")
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("");

                    let finish_reason = choice
                        .get("finish_reason")
                        .and_then(|v| v.as_str());

                    if !delta_text.is_empty() {
                        return Some(format!(
                            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{}\"}}}}\n\n",
                            escape_json_string(delta_text)
                        ));
                    }

                    if let Some(reason) = finish_reason {
                        let stop_reason = match reason {
                            "stop" => "end_turn",
                            "length" => "max_tokens",
                            _ => "end_turn",
                        };
                        return Some(format!(
                            "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{}\",\"stop_sequence\":null}}}}\n\n",
                            stop_reason
                        ));
                    }
                }
            }
        }
    }

    None
}

fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:x}{:x}", duration.as_secs(), duration.subsec_nanos())
}
