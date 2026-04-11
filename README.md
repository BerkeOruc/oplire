```
   ___      ___    _       ___     ___     ___   
  / _ \    | _ \  | |     |_ _|   | _ \   | __|  
 | (_) |   |  _/  | |__    | |    |   /   | _|   
  \___/   _|_|_   |____|  |___|   |_|_\   |___|  
_|"""""|_| """ |_|"""""|_|"""""|_|"""""|_|"""""| 
"`-0-0-'"`-0-0-'"`-0-0-'"`-0-0-'"`-0-0-'"`-0-0-'
     OpenCode Limit Reset Tool              
     by Berke Oruc                     
```

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![AUR](https://img.shields.io/badge/AUR-1.0.4-blue?style=flat-square)](https://aur.archlinux.org/packages/oplire)

## What is oplire?

**oplire** resets your OpenCode session rate limit by managing Cloudflare WARP tunnel connections.

### How It Works

OpenCode tracks users by their IP address. When you hit the rate limit, the tool:

1. **Stops** the current WARP tunnel
2. **Clears** the cached session data  
3. **Creates** a new tunnel registration (new IP)
4. **Restarts** WARP with a fresh IP address

This gives you a new IP address, resetting your rate limit instantly.

### Why OpenCode?

OpenCode (opencode.com) is an AI coding assistant with generous limits. However, sometimes users hit the rate limit due to:
- Long conversations
- Multiple files edits
- Complex prompts

**oplire** solves this by rotating your IP via Cloudflare WARP - no waiting required!

## Installation

### Linux (AUR)
```bash
yay -S oplire
```

### Windows (winget)
```powershell
winget install BerkeOruc.oplire
```

### macOS
```bash
brew install berkeoruc/oplire/oplire
```

### From Source
```bash
git clone https://github.com/BerkeOruc/oplire.git
cd oplire
cargo build --release
sudo cp target/release/oplire /usr/bin/oplire
```

## Usage

### Reset Rate Limit (New IP)
```bash
oplire reset
```

### Stop WARP
```bash
oplire stop
```

### Check Status
```bash
oplire status
```

### Show Info
```bash
oplire about
```

### Install WARP
```bash
oplire install
```

## Options

- `--verbose` - Detailed output
- `--dry-run` - Preview changes without executing
- `--json` - JSON output (status command)

## About

```
Version: 1.0.4
Language: Rust
Purpose: OpenCode rate limit reset
Infrastructure: Cloudflare WARP
Author: Berke Oruc
GitHub: https://github.com/BerkeOruc/oplire
```

## License

MIT License. See [LICENSE](LICENSE) for details.

---

Made by [Berke Oruc](https://github.com/BerkeOruc)