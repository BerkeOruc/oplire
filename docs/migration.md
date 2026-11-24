# Upgrade from oplire 2.x

[Back to README](../README.md)

| If you used | In 3.0 |
| --- | --- |
| `reset`, `quick-reset`, or `watch` | Removed. The bridge returns provider `429` responses and any `Retry-After` header to the caller. It does not reconnect WARP or retry the request. |
| `--max-retries` or `--warp-delay` | Remove the flags from scripts and launch commands. Existing config files can still be read; the old keys are ignored. |
| `daemon` | Still starts the bridge in the foreground. It surfaces rate limits to the caller, just like `proxy`. |
| `status` or `stop` | Use `warp status` or `warp disconnect`. The old names still work. `stop` only disconnects; it leaves the service enabled. |
| Scripts parsing WARP status | Text output now comes from `warp-cli`. Switch to `status --json` and read `connected` or `state` instead of `tunnel_id`. Handle exit status 1 when `warp-cli` is missing or fails. |
| `install warp` | Follow the Cloudflare setup link printed for your platform. `setup` and `install all` no longer install WARP. |
| `opencode serve` as the bridge upstream | Use an inference API that serves OpenAI Chat Completions. See [choosing an upstream](bridge.md#choose-an-upstream). |
| A non-loopback `--listen` address | Use a loopback address such as `127.0.0.1:8080`. The bridge has no client authentication and cannot listen on an external interface. |

If `oplire stop` in 2.x disabled the WARP service on a systemd-based Linux installation, re-enable it with `sudo systemctl enable --now warp-svc`. If the client no longer has a registration, complete the [official Linux setup](https://developers.cloudflare.com/warp-client/get-started/linux/) before using `oplire warp connect`.

Bridge HTTP errors now preserve provider error status codes where applicable. Responses in Anthropic error format use `error.type` and `error.message` instead of `error.code`. Malformed request JSON returns `400`.

For the OpenCode CLI itself, follow the [OpenCode V1 to V2 migration guide](https://opencode.ai/docs/migrate-v1). Use `/models` inside OpenCode for its current free chat models.
