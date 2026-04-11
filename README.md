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
[![AUR](https://img.shields.io/badge/AUR-0.1.0-blue?style=flat-square)](https://aur.archlinux.org/packages/oplire)

Reset your OpenCode limit by managing Cloudflare WARP tunnel connections.

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
./target/release/oplire status
```

## Usage

### Show Info & ASCII Art
```bash
oplire about
```

### Check WARP Status
```bash
oplire status
```

### Reset for New IP
```bash
oplire reset
```

### Install WARP
```bash
oplire install
```

### Options
- `--verbose` - Detailed output
- `--dry-run` - Preview changes without executing
- `--force` - Skip confirmation
- `--json` - JSON output (status command)

## About

```
Version: 0.1.0
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