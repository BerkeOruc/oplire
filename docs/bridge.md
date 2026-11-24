# Inference bridge

[Back to README](../README.md)

Start with the non-streaming request in the [quick start](../README.md#quick-start). Set `stream: true` to receive Anthropic events instead of a single JSON reply. The upstream must support Chat Completions streaming with `stream_options.include_usage` for streamed requests.

```text
Anthropic client              oplire                    Inference API
POST /v1/messages       →     127.0.0.1:8080      →     POST /v1/chat/completions
```

## Choose an upstream

Pass the API root before `/v1` to `--upstream`. The examples use `http://127.0.0.1:3000` as the address of an inference server you start separately. `opencode serve` is an application API and cannot be used here.

For an upstream that does not require authentication, use `oplire models --upstream http://127.0.0.1:3000` to read the live catalog without starting the bridge. The standalone `models` command does not send a key. For authenticated catalogs, start the bridge with `--api-key` and query its local model endpoint, or pass `--api-key` to the Claude Code launcher.

## Authenticate an upstream

```sh
oplire proxy \
  --upstream https://inference.example.com \
  --api-key "$UPSTREAM_API_KEY"
```

Set `UPSTREAM_API_KEY` in your shell before running this example. The bridge sends it as a bearer token for inference and model requests. The argument may be visible to local processes while oplire is running. The bridge only listens on loopback. It rejects browser-origin requests but does not authenticate local processes or forward their credentials. Configure the upstream key only in oplire.

## Local endpoints

| Local endpoint | Behavior |
| --- | --- |
| `GET /health` | Checks that the bridge is listening. Does not contact the upstream. |
| `GET /v1/models` | Reads and translates the upstream model catalog. |
| `GET /v1/models/{model_id}` | Looks up a model. URL-encode `/` in IDs as `%2F`. |
| `POST /v1/messages` | Translates messages, tool definitions, and tool results. Send `application/json`, up to 32 MiB per request. |
| `POST /v1/chat/completions` | Legacy alias for `/v1/messages`. Accepts Anthropic input only. |

## What translates

- Text, system messages, and streamed text replies. The bridge maps upstream token usage when available. Chat Completions does not identify which stop sequence matched, so `stop_sequence` is always `null`.
- Client-defined functions, tool choice, parallel calls, and tool results. The upstream must include a tool call `index` in each streamed tool delta. The bridge waits for a complete, valid set of arguments before returning the calls. The client, not oplire, decides whether to execute a tool.
- Images from base64 data or an HTTP(S) URL, plain-text documents, and base64 PDFs. The upstream model must accept image and PDF inputs. The bridge does not fetch URLs. Attachments in tool results move to a user message after the tool replies because Chat Completions tool messages carry only text.

The bridge does not implement Anthropic server tools, server-side MCP, file IDs, document citations, extended thinking, safeguards, or structured output. It drops thinking blocks from replayed history rather than forwarding or signing them. Anthropic caching markers and effort settings are not translated. There is no `/v1/messages/count_tokens` endpoint; Claude Code estimates counts when it is absent.

## Claude Code launcher

With `claude` on your `PATH`, the launcher starts a bridge and points Claude Code at it:

```sh
oplire connect claude-code --upstream http://127.0.0.1:3000 --model your-model-id
```

Pass `--api-key` if the upstream requires it. Choose a model that supports function calling and streaming. Omit `--model` to pick from the upstream catalog interactively. The launcher uses a local placeholder credential and tells Claude Code to omit its extended thinking request parameter and most experimental beta features by default. Claude Code settings can override launcher environment variables. Check for saved provider, URL, or model overrides if Claude Code does not use the bridge.

The bridge does not replay provider-specific reasoning data. If your provider requires it to continue tool calls, as [DeepSeek does in thinking mode](https://api-docs.deepseek.com/guides/thinking_mode#tool-calls), choose a model or upstream configuration that runs without thinking.

This is a best-effort integration. [Anthropic does not support routing Claude Code to non-Claude models](https://code.claude.com/docs/en/llm-gateway). Use normal client tool permissions with an upstream you trust. The bridge does not support Claude Code features that require Anthropic server-side safeguards. For eligible paid or Go models from OpenCode Console, use the [official Claude Code setup](https://opencode.ai/docs/console/harnesses/claude-code).

## Troubleshooting

- **The catalog is unavailable or invalid:** Confirm the API root and check that `GET /v1/models` returns JSON with a `data` array. On an authenticated upstream, query through a proxy started with `--api-key`.
- **The provider returns `429`:** Follow `Retry-After` when present. The bridge returns the status and header to the caller without retrying or changing WARP.
- **A request is rejected with `400`, `404`, or `422`:** Check the model ID and request limits. The bridge surfaces `error.message` from small upstream JSON validation responses.
- **Claude Code overestimates the context window:** Set `CLAUDE_CODE_MAX_CONTEXT_TOKENS` for the model's real context limit. If the upstream rejects the output limit, lower `CLAUDE_CODE_MAX_OUTPUT_TOKENS` as well. See [Claude Code model configuration](https://code.claude.com/docs/en/model-config#correct-the-window-for-a-gateway-or-custom-model-id).
- **The local port is in use:** Use `--listen 127.0.0.1:8081` and update the client URL. Keep the address on loopback because the bridge does not authenticate local clients.
- **Local model discovery goes through a system proxy:** The running bridge connects to loopback upstreams directly. For `oplire models` or the interactive model picker, add the local host to `NO_PROXY`.
