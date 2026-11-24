# oplire

oplire manages Cloudflare WARP and translates Anthropic Messages requests to OpenAI Chat Completions for compatible upstreams.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

## Install oplire

Prebuilt binaries are published to [GitHub Releases](https://github.com/BerkeOruc/oplire/releases) when a version is tagged. The workflow builds for Linux x86-64, Windows x86-64, and macOS on Intel and Apple Silicon. CI builds and tests on both the latest stable Ubuntu runner and Ubuntu 22.04. The Linux release binary uses the 22.04 build to retain compatibility with older glibc versions.

Build from source with [rustup](https://rustup.rs), a native C toolchain, and CMake:

```sh
git clone https://github.com/BerkeOruc/oplire.git
cd oplire
cargo install --locked --path .
```

The repository pins the stable Rust toolchain used in CI.

## OpenCode V2 and free models

If you are upgrading OpenCode V1, follow the [migration guide](https://opencode.ai/docs/migrate-v1). On a fresh install with Bun, use `oplire install opencode` or install the [OpenCode V2 CLI](https://opencode.ai/docs/#cli) directly:

```sh
bun install -g --trust @opencode/cli
```

Use `/models` inside OpenCode to see the free chat models available to you. Check the [Console catalog](https://opencode.ai/docs/console/models#free-models) for availability and data policies. Free-tier chat models require OpenCode and are unavailable to other harnesses, including Claude Code. For Claude Code with paid or Go models, see the [official Console setup](https://opencode.ai/docs/console/harnesses/claude-code).

## Claude Code bridge

Use the bridge with an upstream you are authorized to use that serves `POST /v1/chat/completions` and `GET /v1/models`. Set `--upstream` to the API root before `/v1`. The standard `opencode serve` command runs OpenCode's application API and is incompatible with this bridge.

```sh
oplire models --upstream http://127.0.0.1:3000
oplire connect claude-code --upstream http://127.0.0.1:3000 --model your-model-id
```

Both commands assume an inference API is already running at that address. The bridge's model list comes from the upstream and returns an error if the catalog is unavailable or invalid. Keep the default loopback listener: the bridge does not authenticate local clients. The bridge can change the host's WARP connection when an inference request is rate limited.

## Other commands

```sh
oplire status              # Check WARP connection status
oplire stop                # Stop the WARP tunnel
oplire install warp        # Install Cloudflare WARP on supported Linux distributions
oplire doctor              # Check the local tools and upstream model endpoint
oplire --help              # List commands and options
```

## License

[MIT](LICENSE). Created by [Berke Oruc](https://github.com/BerkeOruc).
