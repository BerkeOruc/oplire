use std::collections::HashSet;

use serde_json::{Value, json};

pub fn anthropic_to_opencode_request(body: &Value) -> Result<Value, &'static str> {
    let model = required_string(&body["model"]).ok_or("A model ID is required")?;
    let max_tokens = match body.get("max_tokens") {
        Some(value) => value
            .as_u64()
            .ok_or("max_tokens must be a nonnegative integer")?,
        None => 4096,
    };
    let stream = match body.get("stream") {
        Some(value) => value.as_bool().ok_or("stream must be a boolean")?,
        None => false,
    };
    if !body["thinking"].is_null() && body["thinking"]["type"] != "disabled" {
        return Err("Anthropic extended thinking is not supported");
    }
    if !body["safeguards"].is_null() {
        return Err("Server-side safeguards are not supported by this bridge");
    }
    if !body["context_management"].is_null() || !body["mcp_servers"].is_null() {
        return Err("Anthropic context management and server-side MCP are not supported");
    }
    if !body["output_config"]["format"].is_null() || !body["output_config"]["task_budget"].is_null()
    {
        return Err("Anthropic structured output and task budgets are not supported");
    }
    let mut messages: Vec<Value> = Vec::new();

    let system = text_content(&body["system"])?;
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system}));
    }

    for message in body["messages"]
        .as_array()
        .ok_or("Messages must be an array")?
    {
        let content = &message["content"];
        match message["role"].as_str() {
            Some("user") => append_user_message(&mut messages, content)?,
            Some("assistant") => messages.push(assistant_message(content)?),
            Some("system") => {
                let text = text_content(content)?;
                if !text.is_empty() {
                    messages.push(json!({"role": "system", "content": text}));
                }
            }
            _ => return Err("Message roles must be user, assistant, or system"),
        }
    }

    let mut req = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
        "max_tokens": max_tokens
    });
    if stream {
        req["stream_options"] = json!({"include_usage": true});
    }

    for key in ["temperature", "top_p"] {
        if let Some(value) = body.get(key) {
            req[key] = value.clone();
        }
    }
    if let Some(stop) = body.get("stop_sequences") {
        if !stop
            .as_array()
            .is_some_and(|stop| stop.len() <= 4 && stop.iter().all(Value::is_string))
        {
            return Err("The bridge supports up to four string stop sequences");
        }
        if stop.as_array().is_some_and(|stop| !stop.is_empty()) {
            req["stop"] = stop.clone();
        }
    }
    if let Some(tools) = body.get("tools") {
        let tools = tools.as_array().ok_or("Tools must be an array")?;
        if !tools.is_empty() {
            req["tools"] = tools
                .iter()
                .map(tool_definition)
                .collect::<Result<Vec<_>, _>>()?
                .into();
        }
    }
    if let Some(choice) = body.get("tool_choice") {
        let value = match choice["type"].as_str() {
            Some("auto") => json!("auto"),
            Some("any") => json!("required"),
            Some("none") => json!("none"),
            Some("tool") => {
                let name = tool_name(&choice["name"])?;
                json!({"type": "function", "function": {"name": name}})
            }
            _ => return Err("Unsupported tool_choice"),
        };
        if !req["tools"].is_null() {
            req["tool_choice"] = value;
            if choice["disable_parallel_tool_use"] == true {
                req["parallel_tool_calls"] = json!(false);
            }
        } else if !matches!(value.as_str(), Some("auto" | "none")) {
            return Err("tool_choice requires a tool definition");
        }
    }

    Ok(req)
}

fn text_content(content: &Value) -> Result<String, &'static str> {
    match content {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        Value::Array(blocks) => join_text_blocks(blocks),
        _ => Err("Expected text or an array of text blocks"),
    }
}

fn join_text_blocks(blocks: &[Value]) -> Result<String, &'static str> {
    blocks
        .iter()
        .map(text_block)
        .collect::<Result<Vec<_>, _>>()
        .map(|text| text.join("\n"))
}

fn text_block(block: &Value) -> Result<&str, &'static str> {
    if block["type"] != "text" {
        return Err("This content only supports text blocks");
    }
    block["text"].as_str().ok_or("Invalid text block")
}

fn append_user_message(messages: &mut Vec<Value>, content: &Value) -> Result<(), &'static str> {
    let Some(blocks) = content.as_array() else {
        messages.push(json!({"role": "user", "content": text_content(content)?}));
        return Ok(());
    };
    let result_count = blocks
        .iter()
        .take_while(|block| block["type"] == "tool_result")
        .count();
    let (results, user_blocks) = blocks.split_at(result_count);
    let mut attachments = Vec::new();
    for block in results {
        let id = required_string(&block["tool_use_id"]).ok_or("Tool results need a tool use ID")?;
        let mut text = tool_result_text(&block["content"], id, &mut attachments)?;
        if block["is_error"] == true {
            text = format!("Tool failed:\n{text}");
        }
        messages.push(json!({"role": "tool", "tool_call_id": id, "content": text}));
    }

    if !user_blocks.is_empty() || results.is_empty() || !attachments.is_empty() {
        let content =
            if attachments.is_empty() && user_blocks.iter().all(|block| block["type"] == "text") {
                json!(join_text_blocks(user_blocks)?)
            } else {
                for block in user_blocks {
                    match block["type"].as_str() {
                        Some("text") => {
                            attachments.push(json!({"type": "text", "text": text_block(block)?}))
                        }
                        Some("image" | "document") => append_attachment(&mut attachments, block)?,
                        Some("tool_result") => {
                            return Err("Tool results must come before user content");
                        }
                        _ => return Err("Unsupported user content block"),
                    }
                }
                Value::Array(attachments)
            };
        messages.push(json!({"role": "user", "content": content}));
    }
    Ok(())
}

fn tool_result_text(
    content: &Value,
    id: &str,
    attachments: &mut Vec<Value>,
) -> Result<String, &'static str> {
    let Some(blocks) = content.as_array() else {
        return text_content(content);
    };
    let mut text = Vec::new();
    let before = attachments.len();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => text.push(text_block(block)?),
            Some("image" | "document") => {
                if attachments.len() == before {
                    attachments.push(
                        json!({"type": "text", "text": format!("Attachments from tool result {id}:")}),
                    );
                }
                append_attachment(attachments, block)?;
            }
            _ => return Err("Unsupported tool result content block"),
        }
    }
    if attachments.len() != before {
        text.push("[Attachments in the next user message]");
    }
    Ok(text.join("\n"))
}

fn append_attachment(parts: &mut Vec<Value>, block: &Value) -> Result<(), &'static str> {
    let source = &block["source"];
    if block["type"] == "image" {
        parts.push(image_part(source)?);
        return Ok(());
    }
    if block["citations"]["enabled"] == true {
        return Err("Document citations are not supported");
    }
    if let Some(context) = required_string(&block["context"]) {
        parts.push(json!({"type": "text", "text": context}));
    }
    let title = required_string(&block["title"]);
    let part = match (source["type"].as_str(), source["media_type"].as_str()) {
        (Some("text"), Some("text/plain")) => {
            let data = source["data"].as_str().ok_or("Document text is required")?;
            let text = title.map_or_else(|| data.into(), |title| format!("{title}\n{data}"));
            json!({"type": "text", "text": text})
        }
        (Some("base64"), Some("application/pdf")) => {
            let data = required_string(&source["data"]).ok_or("PDF data is required")?;
            json!({"type": "file", "file": {
                "filename": title.unwrap_or("document.pdf"),
                "file_data": format!("data:application/pdf;base64,{data}")
            }})
        }
        _ => return Err("Documents must be plain text or base64 PDFs"),
    };
    parts.push(part);
    Ok(())
}

fn image_part(source: &Value) -> Result<Value, &'static str> {
    let url = match source["type"].as_str() {
        Some("url") => {
            let url = required_string(&source["url"]).ok_or("An image URL is required")?;
            if !url::Url::parse(url)
                .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host().is_some())
            {
                return Err("Image URLs must use HTTP or HTTPS");
            }
            url.to_owned()
        }
        Some("base64") => {
            let media = source["media_type"].as_str().unwrap_or("");
            if !matches!(
                media,
                "image/jpeg" | "image/png" | "image/gif" | "image/webp"
            ) {
                return Err("Unsupported image media type");
            }
            let data = required_string(&source["data"]).ok_or("Image data is required")?;
            format!("data:{media};base64,{data}")
        }
        _ => return Err("Unsupported image source"),
    };
    Ok(json!({"type": "image_url", "image_url": {"url": url}}))
}

fn assistant_message(content: &Value) -> Result<Value, &'static str> {
    let Some(blocks) = content.as_array() else {
        return Ok(json!({"role": "assistant", "content": text_content(content)?}));
    };
    let mut text = Vec::new();
    let mut calls = Vec::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => text.push(text_block(block)?),
            Some("thinking" | "redacted_thinking") => {}
            Some("tool_use") => {
                let id = required_string(&block["id"]).ok_or("Tool calls require an ID")?;
                let name = tool_name(&block["name"])?;
                if !block["input"].is_object() {
                    return Err("Tool input must be an object");
                }
                calls.push(json!({"id": id, "type": "function", "function": {
                    "name": name, "arguments": block["input"].to_string()
                }}));
            }
            _ => return Err("Unsupported assistant content block"),
        }
    }
    if calls.is_empty() {
        Ok(json!({"role": "assistant", "content": text.join("\n")}))
    } else {
        Ok(
            json!({"role": "assistant", "content": (!text.is_empty()).then(|| text.join("\n")), "tool_calls": calls}),
        )
    }
}

fn tool_definition(tool: &Value) -> Result<Value, &'static str> {
    if !tool["type"].is_null() && tool["type"] != "custom" {
        return Err("Only client-defined function tools are supported");
    }
    if tool["defer_loading"] == true {
        return Err("Deferred tool loading is not supported");
    }
    let name = tool_name(&tool["name"])?;
    let parameters = &tool["input_schema"];
    if !parameters.is_object() {
        return Err("Tool input_schema must be an object");
    }
    let mut function = json!({"name": name, "parameters": parameters});
    if let Some(description) = tool["description"].as_str() {
        function["description"] = json!(description);
    }
    if let Some(strict) = tool["strict"].as_bool() {
        function["strict"] = json!(strict);
    }
    Ok(json!({"type": "function", "function": function}))
}

pub(super) fn required_string(value: &Value) -> Option<&str> {
    value.as_str().filter(|value| !value.is_empty())
}

pub(super) fn tool_name(name: &Value) -> Result<&str, &'static str> {
    required_string(name)
        .filter(|name| {
            name.len() <= 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
        .ok_or("Function names must be 1 to 64 letters, digits, underscores, or hyphens")
}

pub(super) fn tool_input(arguments: &str) -> Result<(&str, Value), &'static str> {
    let arguments = if arguments.trim().is_empty() {
        "{}"
    } else {
        arguments
    };
    serde_json::from_str::<Value>(arguments)
        .ok()
        .filter(Value::is_object)
        .map(|input| (arguments, input))
        .ok_or("Upstream tool arguments are not a valid JSON object")
}

pub(super) fn tool_use(id: &str, name: &str, input: &Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

pub fn opencode_response_to_anthropic(body: &Value, model: &str) -> Result<Value, &'static str> {
    let choice = body["choices"]
        .as_array()
        .and_then(|choices| {
            choices
                .iter()
                .find(|choice| choice["index"].is_null() || choice["index"] == 0)
        })
        .ok_or("Upstream response has no completion choice")?;
    let message = &choice["message"];
    if !message.is_object() || !message["function_call"].is_null() {
        return Err("Upstream response has no supported completion message");
    }
    let mut content = Vec::new();
    if let Some(text) =
        required_string(&message["content"]).or_else(|| required_string(&message["refusal"]))
    {
        content.push(json!({"type": "text", "text": text}));
    }
    let mut ids = HashSet::new();
    if !message["tool_calls"].is_null() {
        let calls = message["tool_calls"]
            .as_array()
            .ok_or("Invalid upstream tool calls")?;
        for call in calls {
            if call["type"] != "function" {
                return Err("Upstream returned an unsupported tool type");
            }
            let id = required_string(&call["id"]).ok_or("Upstream tool call has no ID")?;
            if !ids.insert(id) {
                return Err("Upstream returned duplicate tool call IDs");
            }
            let name = tool_name(&call["function"]["name"])?;
            let arguments = call["function"]["arguments"]
                .as_str()
                .ok_or("Upstream tool call has no arguments")?;
            let (_, input) = tool_input(arguments)?;
            content.push(tool_use(id, name, &input));
        }
    }
    let finish_reason = stop_reason(
        choice["finish_reason"].as_str(),
        !ids.is_empty(),
        required_string(&message["refusal"]).is_some(),
    )?;

    Ok(json!({
        "id": message_id(),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": model,
        "stop_reason": finish_reason,
        "stop_sequence": null,
        "usage": token_usage(&body["usage"])
    }))
}

pub(super) fn stop_reason(
    reason: Option<&str>,
    has_tools: bool,
    has_refusal: bool,
) -> Result<&'static str, &'static str> {
    if has_refusal {
        return if has_tools {
            Err("Upstream returned tool calls with a refusal")
        } else {
            Ok("refusal")
        };
    }
    match (reason, has_tools) {
        (Some("function_call"), _) => Err("Upstream used the deprecated function_call format"),
        (Some("tool_calls"), false) => Err("Upstream finished without returning its tool calls"),
        (Some("length" | "content_filter"), true) => Err("Upstream tool calls were interrupted"),
        (Some("length"), false) => Ok("max_tokens"),
        (Some("content_filter"), false) => Ok("refusal"),
        (_, true) => Ok("tool_use"),
        (_, false) => Ok("end_turn"),
    }
}

pub(super) fn token_usage(usage: &Value) -> Value {
    json!({
        "input_tokens": usage["prompt_tokens"].as_u64().unwrap_or(0),
        "output_tokens": usage["completion_tokens"].as_u64().unwrap_or(0)
    })
}

pub(super) fn message_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "msg_{:x}{:09x}",
        duration.as_secs(),
        duration.subsec_nanos()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_images_after_all_tool_results() {
        let request = anthropic_to_opencode_request(&json!({
            "model": "fixture",
            "messages": [{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"call_a","content":[
                    {"type":"text","text":"Screenshot"},
                    {"type":"image","source":{"type":"base64","media_type":"image/png","data":"YQ=="}}
                ]},
                {"type":"tool_result","tool_use_id":"call_b","is_error":true,"content":"missing"},
                {"type":"image","source":{"type":"url","url":"https://example.com/image.png"}},
                {"type":"text","text":"Compare them."}
            ]}]
        })).unwrap();
        let messages = request["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "tool");
        assert_eq!(messages[0]["tool_call_id"], "call_a");
        assert_eq!(
            messages[1],
            json!({"role":"tool","tool_call_id":"call_b","content":"Tool failed:\nmissing"})
        );
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(
            messages[2]["content"][1]["image_url"]["url"],
            "data:image/png;base64,YQ=="
        );
        assert_eq!(
            messages[2]["content"][2]["image_url"]["url"],
            "https://example.com/image.png"
        );
        assert_eq!(messages[2]["content"][3]["text"], "Compare them.");
    }

    #[test]
    fn translates_portable_documents_and_omits_signed_thinking_from_history() {
        let request = anthropic_to_opencode_request(&json!({
            "model":"fixture", "messages":[
                {"role":"assistant","content":[
                    {"type":"thinking","thinking":"hidden","signature":"fixture"},
                    {"type":"redacted_thinking","data":"fixture"},
                    {"type":"text","text":"Send files."}
                ]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"fixture","content":[
                        {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"YQ=="}}
                    ]},
                    {"type":"document","title":"Notes","context":"Review notes", "source":{"type":"text","media_type":"text/plain","data":"hello"}}
                ]}
            ]
        })).unwrap();
        let messages = &request["messages"];
        assert_eq!(messages[0]["content"], "Send files.");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(
            messages[2]["content"][1],
            json!({"type":"file","file":{"file_data":"data:application/pdf;base64,YQ==","filename":"document.pdf"}})
        );
        assert_eq!(messages[2]["content"][2]["text"], "Review notes");
        assert_eq!(messages[2]["content"][3]["text"], "Notes\nhello");
    }

    #[test]
    fn translates_blank_tool_arguments_as_an_empty_object() {
        for arguments in ["", " \t\n"] {
            let response = opencode_response_to_anthropic(
                &json!({"choices": [{"finish_reason": "tool_calls", "message": {
                    "tool_calls": [{"id": "call_date", "type": "function", "function": {
                        "name": "get_date", "arguments": arguments
                    }}]
                }}]}),
                "fixture-model",
            )
            .unwrap();
            assert_eq!(response["content"][0]["input"], json!({}));
            assert_eq!(response["stop_reason"], "tool_use");
        }
    }

    #[test]
    fn rejects_incompatible_features_instead_of_silently_dropping_content() {
        for (key, value) in [
            ("thinking", json!({"type":"adaptive"})),
            ("safeguards", json!({})),
            (
                "tools",
                json!([{"name":"web_search","type":"web_search_20260209"}]),
            ),
            (
                "messages",
                json!([{"role":"user","content":[{"type":"document"}]}]),
            ),
            (
                "messages",
                json!([{"role":"user","content":[{"type":"image","source":{"type":"file","file_id":"file_fixture"}}]}]),
            ),
            (
                "messages",
                json!([{"role":"user","content":[{"type":"text","text":"user text"},{"type":"tool_result","tool_use_id":"call_a"}]}]),
            ),
        ] {
            let mut request = json!({"model":"fixture","messages":[]});
            request[key] = value;
            assert!(anthropic_to_opencode_request(&request).is_err(), "{key}");
        }
    }
}
