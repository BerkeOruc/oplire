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
[![Build Status](https://img.shields.io/github/actions/workflow/status/berkeoruc/oplire-reset/main.yml?style=flat-square&logo=github)](https://github.com/berkeoruc/oplire-reset/actions)

Reset your OpenCode limit by managing Cloudflare WARP tunnel connections.

## Quick Start

### Linux / macOS

```bash
# Clone and build
git clone https://github.com/berkeoruc/oplire-reset.git
cd oplire-reset
cargo build --release

# Run directly
./target/release/oplire-reset status
./target/release/oplire-reset reset
./target/release/oplire-reset install
```

### Windows

```powershell
# Clone and build
git clone https://github.com/berkeoruc/oplire-reset.git
cd oplire-reset
cargo build --release

# Run
.\target\release\oplire-reset.exe status
.\target\release\oplire-reset.exe reset
.\target\release\oplire-reset.exe install
```

## Features

- Check WARP tunnel connection status
- Reset tunnel to refresh your connection
- Install and configure WARP tunnel
- Verbose output mode for debugging
- Dry run mode to preview changes

## Usage

### Check Status

```bash
oplire-reset status
```

Output:
```
Tunnel: Not connected
WARP: Inactive
```

With JSON:
```bash
oplire-reset status --json
```

Output:
```
{"connected": false, "tunnel_id": null}
```

### Reset Connection

```bash
oplire-reset reset
```

To skip confirmation:
```bash
oplire-reset reset --force
```

### Install Tunnel

```bash
oplire-reset install
```

To reinstall even if already present:
```bash
oplire-reset install --force
```

## Install

### Windows (winget)

```powershell
winget install BerkeOruc.oplire
```

Or manually: Download from [Releases](https://github.com/BerkeOruc/oplire/releases)

### Linux (AUR)

```bash
# With yay
yay -S oplire

# Or manually
git clone https://aur.archlinux.org/oplire.git
cd oplire
makepkg -si
```

### macOS

```bash
brew install berkeoruc/oplire/oplire
```

Or from [Releases](https://github.com/BerkeOruc/oplire/releases)

## License

MIT License. See [LICENSE](LICENSE) for details.

---

Made by [Berke Oruc](https://github.com/berkeoruc)
