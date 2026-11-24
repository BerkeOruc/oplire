# Inference bridge

[Back to README](../README.md)

The bridge accepts text in the Anthropic Messages format, forwards it to the configured OpenAI Chat Completions endpoint, and translates the text reply. It does not translate tool definitions, tool calls or results, images, or thinking blocks. Streaming emits text deltas and stop events but omits the Anthropic start events. Clients that require the full stream can show an empty reply or fail. Start with the non-streaming request in the [quick start](../README.md#quick-start).

```text
Anthropic client              oplire                    Inference API
POST /v1/messages       →     127.0.0.1:8080      →     POST /v1/chat/completions
```

## Choose an upstream

Pass the API root before `/v1` to `--upstream`. The examples use `http://127.0.0.1:3000` as the address of an inference server you start separately. `opencode serve` is an application API and cannot be used here.

For an upstream that does not require authentication, use `oplire models --upstream http://127.0.0.1:3000` to read the live catalog without starting the bridge. The `models` command and the launcher's interactive picker do not send an upstream API key. If authentication is required, start the bridge with `--api-key` and query its local model endpoint instead.

## Authenticate an upstream

```sh
oplire proxy \
  --upstream https://inference.example.com \
  --api-key "$UPSTREAM_API_KEY"
```

Set `UPSTREAM_API_KEY` in your shell before running this example. The bridge sends it as a bearer token for inference and model requests. The argument may be visible to local processes while oplire is running. Keep your upstream credentials out of local clients.

## Local endpoints

| Local endpoint | Behavior |
| --- | --- |
| `GET /health` | Checks that the bridge is listening. Does not contact the upstream. |
| `GET /v1/models` | Reads and translates the upstream model catalog. |
| `GET /v1/models/{model_id}` | Looks up a model. URL-encode `/` in IDs as `%2F`. |
| `POST /v1/messages` | Translates Anthropic text requests and replies. |
| `POST /v1/chat/completions` | Legacy alias for `/v1/messages`. Accepts Anthropic input only. |

## Experimental Claude Code launcher

With `claude` on your `PATH`, the launcher starts a bridge and points Claude Code at it:

```sh
oplire connect claude-code --upstream http://127.0.0.1:3000 --model your-model-id
```

Pass `--api-key` if the upstream requires it, and choose a model ID from the upstream catalog. Stream compatibility is incomplete, and Claude Code cannot use file, shell, or other tools through this bridge. For paid or Go models from OpenCode Console, use the [official Claude Code setup](https://opencode.ai/docs/console/harnesses/claude-code).

## Troubleshooting

- **The catalog is unavailable or invalid:** Confirm the API root and check that `GET /v1/models` returns JSON with a `data` array. On an authenticated upstream, query through a proxy started with `--api-key`.
- **The provider returns `429`:** Follow `Retry-After` when present. The bridge returns the status and header to the caller without retrying or changing WARP.
- **A request is rejected with `400`, `404`, or `422`:** Check the model ID and request limits. The bridge preserves `error.message` from small JSON validation responses.
- **The local port is in use:** Use `--listen 127.0.0.1:8081` and update the client URL. Keep the address on loopback because the bridge does not authenticate local clients.
- **Local model discovery goes through a system proxy:** The running bridge connects to loopback upstreams directly. For `oplire models` or the interactive model picker, add the local host to `NO_PROXY`.
