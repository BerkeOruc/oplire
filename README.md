# oplire

oplire translates Anthropic Messages requests to an OpenAI Chat Completions upstream you choose. It supports streaming text, client tools, and image inputs, and includes optional Cloudflare WARP controls.

[Install](#install) · [Quick start](#quick-start) · [Bridge guide](docs/bridge.md) · [WARP guide](docs/warp.md) · [Upgrade from 2.x](docs/migration.md)

## Install

Download the binary for your platform from [GitHub Releases](https://github.com/BerkeOruc/oplire/releases). If the 3.0 release is not listed yet, build from source below. Put the binary on your `PATH` as `oplire` (`oplire.exe` on Windows). On Linux or macOS, run `chmod +x oplire`. Check it with `oplire --version`.

| Platform | Release file |
| --- | --- |
| Linux x86-64 | `oplire-linux` |
| Windows x86-64 | `oplire-windows.exe` |
| macOS Apple Silicon | `oplire-macos-arm` |
| macOS Intel | `oplire-macos` |

The Linux release is built on Ubuntu 22.04 for compatibility with older glibc versions. Releases are published from version tags.

<details>
<summary>Build from source</summary>

Install [rustup](https://rustup.rs), a native C toolchain, and CMake. The project uses Rust edition 2024 and supports Rust 1.88 or newer. The checkout selects the pinned stable compiler used in CI.

```sh
git clone https://github.com/BerkeOruc/oplire.git
cd oplire
cargo install --locked --path .
```

</details>

## Quick start

Start an inference API you are authorized to use. It must serve `POST /v1/chat/completions`; model discovery also needs `GET /v1/models`. Set `--upstream` to the API root **before** `/v1`.

```sh
oplire proxy --upstream http://127.0.0.1:3000
```

In another terminal, list the available models and replace `your-model-id` with an ID from the list:

```sh
curl --fail-with-body http://127.0.0.1:8080/v1/models
curl --fail-with-body http://127.0.0.1:8080/v1/messages \
  -H 'Content-Type: application/json' \
  -d '{"model":"your-model-id","max_tokens":32,"messages":[{"role":"user","content":"Say hello."}]}'
```

See the [bridge guide](docs/bridge.md) for upstream authentication, endpoint details, and troubleshooting.

> [!IMPORTANT]
> The bridge listens only on loopback and does not authenticate local clients. For Claude Code, choose an upstream model with streaming and function calling. See the [launcher guide](docs/bridge.md#claude-code-launcher) for limitations.

## OpenCode V2

With [Bun](https://bun.sh/) on your `PATH`, run `oplire install opencode` or install the CLI directly with `bun install -g --trust @opencode/cli`. See the [V1 migration guide](https://opencode.ai/docs/migrate-v1) if you are upgrading.

Use `/models` in OpenCode to see the current free chat models. The [Console catalog](https://opencode.ai/docs/console/models#free-models) lists availability and data policies. Those models require OpenCode and cannot be used through the bridge. `opencode serve` exposes the OpenCode application API, which is incompatible with the bridge. For eligible paid or Go models in Claude Code, see the [official Console setup](https://opencode.ai/docs/console/harnesses/claude-code).

## Cloudflare WARP

WARP is optional. `oplire warp status`, `oplire warp connect`, and `oplire warp disconnect` use the client you already installed and set up.

See [WARP setup and troubleshooting](docs/warp.md) or [what changed in 3.0](docs/migration.md).

## License

[MIT](LICENSE). Created by [Berke Oruc](https://github.com/BerkeOruc).
