# Cloudflare WARP

[Back to README](../README.md)

WARP is optional. The bridge works without it, and connections to loopback upstreams stay local. Install the official client for [Linux](https://developers.cloudflare.com/warp-client/get-started/linux/), [macOS](https://developers.cloudflare.com/warp-client/get-started/macos/), or [Windows](https://developers.cloudflare.com/warp-client/get-started/windows/). `oplire install warp` prints the setup link for your platform. Complete first-time setup in the Cloudflare app or CLI before using oplire, which cannot answer setup prompts.

If `warp-cli` is available on your `PATH`, use:

```sh
oplire warp status
oplire warp connect
oplire warp disconnect
```

Connect and disconnect affect the host's WARP connection. They use the existing enrollment and do not change the client's mode or disable system services. Add `--dry-run` to preview a connect or disconnect without running it.

## Status and troubleshooting

- **`warp-cli` was not found:** Use the official client app, or add the folder containing `warp-cli` to your `PATH` if the CLI is installed.
- **The client rejects a command:** Check the client's status and enrollment in the official app or CLI. Managed devices may restrict connection changes.
- **Check status in a script:** `oplire warp status --json` prints a single object. `state` is `connected`, `disconnected`, `connecting`, or `unknown`. `connected` is true only for `connected`. If the client is missing or the status command fails, the output includes `error` and oplire exits with status 1.

```json
{"connected":false,"state":"disconnected"}
```

Status reports the local client's state. It does not verify which route a particular inference request took.
