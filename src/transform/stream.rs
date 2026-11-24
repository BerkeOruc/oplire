use std::{
    collections::BTreeMap,
    fmt::Write,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::anthropic::{
    message_id, required_string, stop_reason, token_usage, tool_input, tool_name, tool_use,
};

const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_TOOL_BYTES: usize = 4 * MAX_EVENT_BYTES;
const MAX_TOOL_CALLS: usize = 128;

#[derive(Default)]
struct SseData {
    line: Vec<u8>,
    data: Vec<u8>,
    skip_lf: bool,
    saw_line: bool,
}

impl SseData {
    fn next(&mut self, input: &mut &[u8]) -> Result<Option<Vec<u8>>, &'static str> {
        while !input.is_empty() {
            if std::mem::take(&mut self.skip_lf) && input.starts_with(b"\n") {
                *input = &input[1..];
                continue;
            }

            let end = input.iter().position(|&b| matches!(b, b'\r' | b'\n'));
            let (line_part, tail) = input.split_at(end.unwrap_or(input.len()));
            if line_part.len() > MAX_EVENT_BYTES - self.line.len() {
                return Err("Upstream event stream line is too large");
            }
            self.line.extend_from_slice(line_part);
            let Some((&delimiter, rest)) = tail.split_first() else {
                *input = tail;
                break;
            };
            *input = rest;
            self.skip_lf = delimiter == b'\r';

            let mut line = self.line.as_slice();
            if !std::mem::replace(&mut self.saw_line, true) {
                line = line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(line);
            }
            if line.is_empty() && !self.data.is_empty() {
                self.line.clear();
                self.data.pop(); // Remove the final separator between data lines.
                return Ok(Some(std::mem::take(&mut self.data)));
            }
            if let Some(value) = line
                .strip_prefix(b"data:")
                .or_else(|| (line == b"data").then_some(&[]))
            {
                let value = value.strip_prefix(b" ").unwrap_or(value);
                if value.len() >= MAX_EVENT_BYTES - self.data.len() {
                    return Err("Upstream event stream event is too large");
                }
                self.data.extend_from_slice(value);
                self.data.push(b'\n');
            }
            self.line.clear();
        }
        Ok(None)
    }

    fn has_partial_event(&self) -> bool {
        !self.line.is_empty() || !self.data.is_empty()
    }
}

#[derive(Default)]
struct ToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn set_once(field: &mut Option<String>, value: &str) -> Result<(), &'static str> {
    match field {
        Some(existing) if existing != value => Err("Upstream changed tool call metadata"),
        Some(_) => Ok(()),
        None => {
            *field = Some(value.into());
            Ok(())
        }
    }
}

pub(crate) struct MessageStream {
    frames: SseData,
    model: String,
    usage: Value,
    started: bool,
    text_started: bool,
    has_refusal: bool,
    finish_reason: Option<String>,
    tools: BTreeMap<u64, ToolCall>,
    tool_bytes: usize,
    finished: bool,
    last_output: Instant,
}

impl MessageStream {
    pub(crate) fn new(model: &str) -> Self {
        Self {
            frames: SseData::default(),
            model: model.into(),
            usage: token_usage(&Value::Null),
            started: false,
            text_started: false,
            has_refusal: false,
            finish_reason: None,
            tools: BTreeMap::new(),
            tool_bytes: 0,
            finished: false,
            last_output: Instant::now(),
        }
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.finished
    }

    pub(crate) fn push(&mut self, mut input: &[u8]) -> String {
        let mut output = String::new();
        let upstream_active = !input.is_empty();
        while !self.finished {
            let result = match self.frames.next(&mut input) {
                Ok(Some(data)) => self.translate(&data, &mut output),
                Ok(None) => break,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                output.push_str(&self.fail(error));
            }
        }
        if upstream_active
            && output.is_empty()
            && !self.finished
            && self.last_output.elapsed() >= Duration::from_secs(15)
        {
            emit(&mut output, "ping", json!({}));
        }
        if !output.is_empty() {
            self.last_output = Instant::now();
        }
        output
    }

    pub(crate) fn eof(&mut self) -> String {
        if self.started && self.finish_reason.is_some() && !self.frames.has_partial_event() {
            let mut output = String::new();
            match self.complete(&mut output) {
                Ok(()) => output,
                Err(error) => self.fail(error),
            }
        } else {
            self.fail("Upstream event stream ended before completion")
        }
    }

    pub(crate) fn fail(&mut self, message: &str) -> String {
        self.finished = true;
        let mut output = String::new();
        emit(
            &mut output,
            "error",
            json!({"error": {"type": "api_error", "message": message}}),
        );
        output
    }

    fn translate(&mut self, data: &[u8], output: &mut String) -> Result<(), &'static str> {
        if data.is_empty() {
            return Ok(());
        }
        if data == b"[DONE]" {
            if !self.started {
                return Err("Upstream event stream ended without a completion");
            }
            return self.complete(output);
        }

        let chunk: Value =
            serde_json::from_slice(data).map_err(|_| "Invalid upstream event stream JSON")?;
        if !chunk["error"].is_null() {
            return Err("Upstream returned an error during streaming");
        }
        if chunk["usage"].is_object() {
            self.usage = token_usage(&chunk["usage"]);
        }
        let choices = match chunk["choices"].as_array() {
            Some(choices) => choices,
            None if chunk["choices"].is_null() && chunk["usage"].is_object() => return Ok(()),
            None => return Err("Upstream event is missing its choices"),
        };
        let Some(choice) = choices
            .iter()
            .find(|choice| choice["index"].is_null() || choice["index"] == 0)
        else {
            return Ok(());
        };
        if !choice.is_object() {
            return Err("Invalid upstream completion choice");
        }
        let delta = &choice["delta"];
        if self.finish_reason.is_some() {
            if (choice["finish_reason"].is_null()
                || choice["finish_reason"].as_str() == self.finish_reason.as_deref())
                && required_string(&delta["content"]).is_none()
                && required_string(&delta["refusal"]).is_none()
                && delta["function_call"].is_null()
                && (delta["tool_calls"].is_null()
                    || delta["tool_calls"].as_array().is_some_and(Vec::is_empty))
            {
                return Ok(());
            }
            return Err("Upstream sent a completion choice after the finish reason");
        }
        if !delta["function_call"].is_null() || choice["finish_reason"] == "function_call" {
            return Err("Upstream used the deprecated function_call format");
        }
        if !delta["tool_calls"].is_null() {
            let calls = delta["tool_calls"]
                .as_array()
                .ok_or("Invalid upstream tool calls")?;
            for call in calls {
                self.append_tool_call(call)?;
            }
        }

        if !self.started {
            self.started = true;
            emit(
                output,
                "message_start",
                json!({
                    "message": {
                        "id": message_id(), "type": "message", "role": "assistant",
                        "content": [], "model": self.model,
                        "stop_reason": null, "stop_sequence": null, "usage": self.usage
                    }
                }),
            );
        }
        let refusal = required_string(&delta["refusal"]);
        self.has_refusal |= refusal.is_some();
        if let Some(text) = required_string(&delta["content"]).or(refusal) {
            if !self.text_started {
                self.text_started = true;
                emit(
                    output,
                    "content_block_start",
                    json!({
                        "index": 0, "content_block": {"type": "text", "text": ""}
                    }),
                );
            }
            emit(
                output,
                "content_block_delta",
                json!({
                    "index": 0, "delta": {"type": "text_delta", "text": text}
                }),
            );
        }
        self.finish_reason = choice["finish_reason"].as_str().map(str::to_owned);
        Ok(())
    }

    fn append_tool_call(&mut self, call: &Value) -> Result<(), &'static str> {
        let index = call["index"]
            .as_u64()
            .ok_or("Upstream tool call has no index")?;
        if index >= MAX_TOOL_CALLS as u64 {
            return Err("Upstream tool call index exceeds the bridge limit");
        }
        if !call["type"].is_null() && call["type"] != "function" {
            return Err("Upstream returned an unsupported tool type");
        }
        let function = &call["function"];
        if !function.is_null() && !function.is_object() {
            return Err("Invalid upstream tool function");
        }
        let tool = self.tools.entry(index).or_default();
        if !call["id"].is_null() && call["id"] != "" {
            let id = required_string(&call["id"])
                .filter(|id| id.len() <= 256)
                .ok_or("Invalid upstream tool call ID")?;
            set_once(&mut tool.id, id)?;
        }
        if !function["name"].is_null() && function["name"] != "" {
            set_once(&mut tool.name, tool_name(&function["name"])?)?;
        }
        if !function["arguments"].is_null() {
            let arguments = function["arguments"]
                .as_str()
                .ok_or("Invalid upstream tool arguments")?;
            if arguments.len() > MAX_TOOL_BYTES - self.tool_bytes {
                return Err("Upstream tool arguments exceed the bridge limit");
            }
            self.tool_bytes += arguments.len();
            tool.arguments.push_str(arguments);
        }
        Ok(())
    }

    fn complete(&mut self, output: &mut String) -> Result<(), &'static str> {
        if !self.tools.is_empty() && self.finish_reason.is_none() {
            return Err("Upstream ended before finishing its tool calls");
        }
        let reason = stop_reason(
            self.finish_reason.as_deref(),
            !self.tools.is_empty(),
            self.has_refusal,
        )?;
        let mut ids = std::collections::HashSet::new();
        // Validate every call before publishing any tool block. A partial group
        // must not look executable to the client.
        let tools = self
            .tools
            .values()
            .map(|tool| {
                let id = tool.id.as_deref().ok_or("Upstream tool call has no ID")?;
                if !ids.insert(id) {
                    return Err("Upstream returned duplicate tool call IDs");
                }
                let name = tool
                    .name
                    .as_deref()
                    .ok_or("Upstream tool call has no name")?;
                let (arguments, _) = tool_input(&tool.arguments)?;
                Ok((id, name, arguments))
            })
            .collect::<Result<Vec<_>, _>>()?;

        if self.text_started {
            emit(output, "content_block_stop", json!({"index": 0}));
        }
        for (index, (id, name, arguments)) in tools.into_iter().enumerate() {
            let index = index + usize::from(self.text_started);
            emit(
                output,
                "content_block_start",
                json!({"index": index, "content_block": tool_use(id, name, &json!({}))}),
            );
            emit(
                output,
                "content_block_delta",
                json!({"index": index, "delta": {"type": "input_json_delta", "partial_json": arguments}}),
            );
            emit(output, "content_block_stop", json!({"index": index}));
        }
        emit(
            output,
            "message_delta",
            json!({
                "delta": {"stop_reason": reason, "stop_sequence": null},
                "usage": self.usage
            }),
        );
        emit(output, "message_stop", json!({}));
        self.finished = true;
        Ok(())
    }
}

fn emit(output: &mut String, event: &str, mut data: Value) {
    data["type"] = json!(event);
    write!(output, "event: {event}\ndata: {data}\n\n").expect("writing to a String cannot fail");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(output: &str) -> Vec<Value> {
        output
            .trim_end()
            .split("\n\n")
            .map(|event| {
                let (name, data) = event.split_once("\ndata: ").unwrap();
                let value: Value = serde_json::from_str(data).unwrap();
                assert_eq!(value["type"], name.strip_prefix("event: ").unwrap());
                value
            })
            .collect()
    }

    #[test]
    fn emits_a_complete_message_when_every_byte_arrives_separately() {
        let fixture = concat!(
            "\u{feff}: keep alive\r\n\r\n",
            "data:{\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\r\n\r\n",
            "data: {\"choices\":\n",
            "data: [{\"delta\":{\"content\":\"Hi 👋\\b\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"!\"},\"finish_reason\":\"length\"}]}\r\r",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\r\n\r\n",
            "data: [DONE]\n\n"
        );
        let mut stream = MessageStream::new("fixture-model");
        let output: String = fixture
            .as_bytes()
            .chunks(1)
            .map(|b| stream.push(b))
            .collect();
        let events = events(&output);
        let names: Vec<_> = events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert_eq!(events[0]["message"]["model"], "fixture-model");
        assert_eq!(events[0]["message"]["content"], json!([]));
        assert_eq!(events[2]["delta"]["text"], "Hi 👋\u{0008}");
        assert_eq!(events[3]["delta"]["text"], "!");
        assert_eq!(events[5]["delta"]["stop_reason"], "max_tokens");
        assert_eq!(
            events[5]["usage"],
            json!({"input_tokens": 7, "output_tokens": 3})
        );
        assert!(stream.is_finished());
    }

    #[test]
    fn reports_malformed_or_truncated_streams_as_errors() {
        let mut stream = MessageStream::new("fixture-model");
        let start = stream.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n");
        assert!(start.contains("event: content_block_delta\n"));
        assert_eq!(events(&stream.eof())[0]["type"], "error");

        let mut stream = MessageStream::new("fixture-model");
        let output = stream.push(b"data: not-json-private-content\n\ndata: [DONE]\n\n");
        assert!(!output.contains("not-json-private-content"));
        assert_eq!(events(&output)[0]["type"], "error");
        assert!(stream.is_finished());
    }

    #[test]
    fn accepts_a_clean_eof_after_a_finish_reason() {
        let mut stream = MessageStream::new("fixture-model");
        stream.push(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n");
        let events = events(&stream.eof());
        assert_eq!(events[0]["delta"]["stop_reason"], "end_turn");
        assert_eq!(events[1]["type"], "message_stop");
    }

    fn chunk(delta: Value, finish: Option<&str>) -> String {
        format!(
            "data: {}\n\n",
            json!({"choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
        )
    }

    #[test]
    fn validates_and_orders_interleaved_tools_after_streaming_text() {
        let mut stream = MessageStream::new("fixture-model");
        let first = chunk(
            json!({"content": "Looking up both.", "tool_calls": [
                {"index": 1, "id": "call_b", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\":"}},
                {"index": 0, "id": "call_a", "type": "function", "function": {"name": "lookup", "arguments": "{\"q\":\"hi"}}
            ]}),
            None,
        );
        let starts = events(&stream.push(first.as_bytes()));
        assert_eq!(starts.len(), 3);
        assert_eq!(starts[2]["delta"]["text"], "Looking up both.");
        let last = chunk(
            json!({"tool_calls": [
                {"index": 1, "function": {"arguments": "\"bye\"}"}},
                {"index": 0, "function": {"arguments": " 👋\"}"}}
            ]}),
            Some("tool_calls"),
        );
        assert!(stream.push(last.as_bytes()).is_empty());
        let done = events(&stream.push(b"data: [DONE]\n\n"));
        assert_eq!(done[0], json!({"type":"content_block_stop","index":0}));
        assert_eq!(done[1]["index"], 1);
        assert_eq!(
            done[1]["content_block"],
            json!({"type": "tool_use", "name": "lookup", "id": "call_a", "input": {}})
        );
        assert_eq!(
            done[2]["delta"],
            json!({"type": "input_json_delta", "partial_json": "{\"q\":\"hi 👋\"}"})
        );
        assert_eq!(done[3], json!({"type":"content_block_stop","index":1}));
        assert_eq!(done[4]["index"], 2);
        assert_eq!(done[4]["content_block"]["id"], "call_b");
        assert_eq!(done[5]["delta"]["partial_json"], "{\"q\":\"bye\"}");
        assert_eq!(done[6], json!({"type":"content_block_stop","index":2}));
        assert_eq!(done[7]["delta"]["stop_reason"], "tool_use");
        assert_eq!(done[8]["type"], "message_stop");
    }

    #[test]
    fn emits_an_empty_object_for_blank_tool_arguments() {
        for arguments in ["", " \t\n"] {
            let mut stream = MessageStream::new("fixture-model");
            let payload = chunk(
                json!({"tool_calls": [{"index": 0, "id": "call_date", "type": "function",
                    "function": {"name": "get_date", "arguments": arguments}}]}),
                Some("tool_calls"),
            );
            stream.push(payload.as_bytes());
            let done = events(&stream.push(b"data: [DONE]\n\n"));
            assert_eq!(done[0]["content_block"]["input"], json!({}));
            assert_eq!(done[1]["delta"]["partial_json"], "{}");
            assert_eq!(done[3]["delta"]["stop_reason"], "tool_use");
        }
    }

    #[test]
    fn never_publishes_an_incomplete_or_invalid_group_of_tool_calls() {
        for (arguments, finish, id) in [
            ("{", Some("tool_calls"), "call_b"),
            ("{}", Some("length"), "call_b"),
            ("{}", None, "call_b"),
            ("{}", Some("tool_calls"), "call_a"),
        ] {
            let mut stream = MessageStream::new("fixture-model");
            let payload = chunk(
                json!({"tool_calls": [
                    {"index": 0, "id": "call_a", "function": {"name": "lookup", "arguments": "{}"}},
                    {"index": 1, "id": id, "function": {"name": "lookup", "arguments": arguments}},
                ]}),
                finish,
            );
            assert_eq!(
                events(&stream.push(payload.as_bytes()))[0]["type"],
                "message_start"
            );
            let done = events(&stream.push(b"data: [DONE]\n\n"));
            assert_eq!(done.len(), 1);
            assert_eq!(done[0]["type"], "error");
        }
    }

    #[test]
    fn streams_refusal_text_without_forwarding_reasoning() {
        let mut stream = MessageStream::new("fixture-model");
        let input = chunk(
            json!({"refusal": "I cannot help.", "reasoning_content": "private reasoning"}),
            Some("stop"),
        );
        let output = stream.push(input.as_bytes());
        assert!(!output.contains("private reasoning"));
        assert_eq!(events(&output)[2]["delta"]["text"], "I cannot help.");
        assert_eq!(events(&stream.eof())[1]["delta"]["stop_reason"], "refusal");
    }

    #[test]
    fn accepts_usage_only_chunks_after_finishing_but_rejects_new_content() {
        for append_content in [false, true] {
            let mut stream = MessageStream::new("fixture-model");
            stream.push(chunk(json!({"content":"done"}), Some("stop")).as_bytes());
            assert!(
                stream
                    .push(chunk(json!({}), Some("stop")).as_bytes())
                    .is_empty()
            );
            assert!(
                stream
                    .push(b"data: {\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1}}\n\n")
                    .is_empty()
            );
            if append_content {
                let output = stream.push(chunk(json!({"content":"extra"}), None).as_bytes());
                assert_eq!(events(&output)[0]["type"], "error");
            } else {
                let done = events(&stream.eof());
                assert_eq!(
                    done[1]["usage"],
                    json!({"input_tokens":5,"output_tokens":1})
                );
            }
        }
    }

    #[test]
    fn throttles_pings_during_quiet_upstream_activity() {
        let mut stream = MessageStream::new("fixture-model");
        stream.push(chunk(json!({"role":"assistant"}), None).as_bytes());
        stream.last_output = Instant::now() - Duration::from_secs(16);
        assert_eq!(
            events(&stream.push(b": upstream keepalive\n\n")),
            [json!({"type":"ping"})]
        );
        assert!(stream.push(b": upstream keepalive\n\n").is_empty());
    }

    #[test]
    fn bounds_unterminated_lines_and_multi_line_events() {
        let mut stream = MessageStream::new("fixture-model");
        assert!(stream.push(&vec![b'x'; MAX_EVENT_BYTES]).is_empty());
        assert_eq!(events(&stream.push(b"x"))[0]["type"], "error");

        let mut stream = MessageStream::new("fixture-model");
        let line = format!("data: {}\n", "x".repeat(MAX_EVENT_BYTES / 2));
        assert!(stream.push(line.as_bytes()).is_empty());
        assert_eq!(events(&stream.push(line.as_bytes()))[0]["type"], "error");
    }
}
