use clap::{Parser, Subcommand};
use colored::Colorize;
use oplire_reset::{ProxyConfig, AppConfig};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const VERSION: &str = "2.1.0";

#[derive(Parser, Debug)]
#[command(name = "oplire")]
#[command(version = VERSION)]
#[command(about = "OpenCode Limit Reset + Anthropic Proxy Bridge", long_about = None)]
struct Cli {
    #[arg(short, long, global = true)]
    verbose: bool,

    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Status {
        #[arg(long)]
        json: bool,
    },
    Reset {},
    QuickReset {},
    Install {},
    Stop {},
    About {},
    Proxy {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long, default_value = "3")]
        max_retries: u32,
        #[arg(long, default_value = "5000")]
        warp_delay: u64,
    },
    Connect {
        #[command(subcommand)]
        target: ConnectTarget,
    },
    Watch {
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
        #[arg(long, default_value = "3")]
        max_retries: u32,
        #[arg(long, default_value = "5000")]
        warp_delay: u64,
    },
    Daemon {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long, default_value = "3")]
        max_retries: u32,
        #[arg(long, default_value = "5000")]
        warp_delay: u64,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Doctor {},
}

#[derive(Subcommand, Debug)]
enum ConnectTarget {
    ClaudeCode {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long, default_value = "3")]
        max_retries: u32,
        #[arg(long, default_value = "5000")]
        warp_delay: u64,
        #[arg(long)]
        model: Option<String>,
        #[arg(last = true)]
        claude_args: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    Show {},
    Set {
        #[arg(long)]
        key: Option<String>,
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
        #[arg(long, default_value = "3")]
        max_retries: u32,
        #[arg(long, default_value = "5000")]
        warp_delay: u64,
    },
    Reset {},
}

fn config_path() -> PathBuf {
    let mut path = dirs_config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("oplire");
    path.push("config.json");
    path
}

fn dirs_config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        if let Ok(home) = std::env::var("HOME") {
            let mut p = PathBuf::from(home);
            p.push("Library");
            p.push("Application Support");
            return Some(p);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    if let Ok(home) = std::env::var("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".config");
        return Some(p);
    }
    None
}

fn load_config() -> AppConfig {
    let path = config_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str(&content) {
                return config;
            }
        }
    }
    AppConfig::default()
}

fn save_config(config: &AppConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(())
}

fn check_warp_installed() -> bool {
    Command::new("warp-cli").arg("--version").output().is_ok()
}

fn check_claude_installed() -> bool {
    Command::new("claude").arg("--version").output().is_ok()
}

fn check_opencode_running(base_url: &str) -> bool {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    reqwest::blocking::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

fn run_command(cmd: &str, args: &[&str], dry_run: bool, verbose: bool) -> Result<String, String> {
    if dry_run {
        if verbose {
            eprintln!(
                "{} Would execute: {} {}",
                "[DRY-RUN]".cyan(),
                cmd,
                args.join(" ")
            );
        } else {
            eprintln!("{} {} {}", "[DRY-RUN]".cyan(), cmd, args.join(" "));
        }
        Ok(format!(
            "[DRY-RUN] Would execute: {} {}",
            cmd,
            args.join(" ")
        ))
    } else {
        let output = Command::new(cmd)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

fn run_sudo_command(cmd: &str, dry_run: bool, verbose: bool) -> Result<String, String> {
    if dry_run {
        if verbose {
            eprintln!("{} Would execute: sudo {}", "[DRY-RUN]".cyan(), cmd);
        } else {
            eprintln!("{} sudo {}", "[DRY-RUN]".cyan(), cmd);
        }
        Ok(format!("[DRY-RUN] Would execute: sudo {}", cmd))
    } else {
        let output = Command::new("sudo")
            .arg("-S")
            .arg("-n")
            .arg("-k")
            .arg("-v")
            .output()
            .map_err(|e| e.to_string())?;

        if !output.status.success() {
            return Err("sudo requires password or -n flag failed".to_string());
        }

        let output = Command::new("sudo")
            .arg("-S")
            .arg("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|e| e.to_string())?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

fn print_banner() {
    println!(
        "{}",
        r#"
  ___      ___    _       ___     ___     ___   
 / _ \    | _ \  | |     |_ _|   | _ \   | __|  
| (_) |   |  _/  | |__    | |    |   /   | _|   
 \___/   _|_|_   |____|  |___|   |_|_\   |___|  
_|"""""|_| """ |_|"""""|_|"""""|_|"""""|_|"""""| 
"`-0-0-'"`-0-0-'"`-0-0-'"`-0-0-'"`-0-0-'"`-0-0-'
"#
        .bold()
        .cyan()
    );
}

fn main() {
    let cli = Cli::parse();

    if cli.verbose {
        eprintln!("{} Verbose mode enabled", "[DEBUG]".yellow());
    }

    if cli.dry_run {
        eprintln!(
            "{} Dry run mode - no changes will be made",
            "[DRY-RUN]".cyan()
        );
    }

    let warp_installed = check_warp_installed();

    match &cli.command {
        Commands::Connect {
            target: ConnectTarget::ClaudeCode {
                listen,
                upstream,
                api_key,
                max_retries,
                warp_delay,
                model,
                claude_args,
            },
        } => {
            print_banner();
            println!("{}", "Claude Code Bridge".bold().green());
            println!();

            if !check_claude_installed() {
                eprintln!("{} Claude Code not found in PATH", "[ERROR]".red());
                eprintln!("{} Install: npm install -g @anthropic-ai/claude-code", "Fix:".cyan());
                std::process::exit(1);
            }

            let config = ProxyConfig {
                listen_addr: listen.clone(),
                opencode_base_url: upstream.clone(),
                opencode_api_key: api_key.clone(),
                max_retries: *max_retries,
                warp_reset_delay_ms: *warp_delay,
            };

            println!("{} Proxy:      {}", "→".green(), listen.bold());
            println!("{} Upstream:   {}", "→".green(), upstream.bold());
            println!("{} Auto-reset: {} (attempts: {})", "→".green(), "enabled".green().bold(), max_retries.to_string().bold());
            if let Some(m) = model {
                println!("{} Model:     {}", "→".green(), m.bold());
            }
            println!();

            println!("{}", "Starting proxy server...".dimmed());

            let proxy_config = config.clone();
            let listen_clone = listen.clone();

            let proxy_handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    oplire_reset::proxy::start_proxy_server(proxy_config).await
                })
            });

            std::thread::sleep(std::time::Duration::from_millis(1500));

            println!("{}", "Launching Claude Code...".dimmed());
            println!();

            let mut cmd = Command::new("claude");
            cmd.env("ANTHROPIC_BASE_URL", format!("http://{}", listen_clone))
                .env("ANTHROPIC_API_KEY", "oplire-proxy-key");

            if let Some(m) = model {
                cmd.env("ANTHROPIC_MODEL", m);
            }

            if !claude_args.is_empty() {
                cmd.args(claude_args);
            }

            let status = cmd.status().map_err(|e| e.to_string()).unwrap_or_else(|_| {
                eprintln!("{} Failed to launch Claude Code", "[ERROR]".red());
                std::process::exit(1);
            });

            println!();
            println!("{} Claude Code exited with: {}", "Info:".cyan(), status.to_string().bold());
            println!("{} Shutting down proxy...", "→".green().dimmed());

            drop(proxy_handle);
        }

        Commands::Watch {
            upstream,
            max_retries,
            warp_delay,
        } => {
            print_banner();
            println!("{}", "OpenCode Watch Mode".bold().yellow());
            println!();
            println!("{} Monitoring: {}", "→".green(), upstream.bold());
            println!("{} Auto-reset: {} (attempts: {})", "→".green(), "enabled".green().bold(), max_retries.to_string().bold());
            println!();
            println!("{}", "Watching for 429 rate limits...".dimmed());
            println!("{} Press Ctrl+C to stop", "Tip:".cyan());
            println!();

            let upstream_clone = upstream.clone();
            let max_retries_clone = *max_retries;
            let warp_delay_clone = *warp_delay;

            let rt = tokio::runtime::Runtime::new().unwrap();
            if let Err(e) = rt.block_on(async {
                oplire_reset::watch::start_watch_mode(
                    &upstream_clone,
                    max_retries_clone,
                    warp_delay_clone,
                )
                .await
            }) {
                eprintln!("{} Watch error: {}", "[ERROR]".red(), e);
                std::process::exit(1);
            }
        }

        Commands::Daemon {
            listen,
            upstream,
            api_key,
            max_retries,
            warp_delay,
        } => {
            print_banner();
            println!("{}", "Daemon Mode".bold().magenta());
            println!();
            println!("{} Listening:  {}", "→".green(), listen.bold());
            println!("{} Upstream:   {}", "→".green(), upstream.bold());
            println!("{} Auto-reset: {} (attempts: {})", "→".green(), "enabled".green().bold(), max_retries.to_string().bold());
            println!();
            println!("{}", "Running as background daemon...".dimmed());
            println!("{} The proxy will auto-reset rate limits silently", "Tip:".cyan());
            println!();

            let config = ProxyConfig {
                listen_addr: listen.clone(),
                opencode_base_url: upstream.clone(),
                opencode_api_key: api_key.clone(),
                max_retries: *max_retries,
                warp_reset_delay_ms: *warp_delay,
            };

            let rt = tokio::runtime::Runtime::new().unwrap();
            if let Err(e) = rt.block_on(async {
                oplire_reset::proxy::start_proxy_server(config).await
            }) {
                eprintln!("{} Daemon error: {}", "[ERROR]".red(), e);
                std::process::exit(1);
            }
        }

        Commands::Doctor {} => {
            print_banner();
            println!("{}", "System Diagnostics".bold().cyan());
            println!();

            let mut all_ok = true;

            print!("{} WARP CLI: ", "→".cyan());
            if warp_installed {
                println!("{}", "installed".green().bold());
            } else {
                println!("{}", "NOT FOUND".red().bold());
                all_ok = false;
            }

            print!("{} Claude Code: ", "→".cyan());
            if check_claude_installed() {
                let version = run_command("claude", &["--version"], false, false)
                    .map(|v| v.trim().to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                println!("{} ({})", "installed".green().bold(), version.dimmed());
            } else {
                println!("{}", "NOT FOUND".red().bold());
                all_ok = false;
            }

            print!("{} OpenCode Zen: ", "→".cyan());
            let config = load_config();
            let upstream = config.upstream.clone();
            if check_opencode_running(&upstream) {
                println!("{} ({})", "running".green().bold(), upstream.dimmed());
            } else {
                println!("{} ({})", "NOT REACHABLE".red().bold(), upstream.dimmed());
                all_ok = false;
            }

            print!("{} Config file: ", "→".cyan());
            let cfg_path = config_path();
            if cfg_path.exists() {
                println!("{} ({})", "found".green().bold(), cfg_path.display().to_string().dimmed());
            } else {
                println!("{} (using defaults)", "not found".yellow().bold());
            }

            print!("{} Port 8080: ", "→".cyan());
            if std::net::TcpListener::bind("127.0.0.1:8080").is_ok() {
                println!("{}", "available".green().bold());
            } else {
                println!("{}", "IN USE".red().bold());
                all_ok = false;
            }

            println!();
            if all_ok {
                println!("{}", "All checks passed!".green().bold());
            } else {
                println!("{}", "Some checks failed. See above for details.".yellow().bold());
                println!();
                println!("{} Run `oplire install` to install WARP", "Fix:".cyan());
                println!("{} Install Claude Code: npm install -g @anthropic-ai/claude-code", "Fix:".cyan());
                println!("{} Ensure OpenCode Zen is running on {}", "Fix:".cyan(), upstream.bold());
            }
        }

        Commands::QuickReset {} => {
            if !warp_installed {
                println!("{} WARP is not installed", "[ERROR]".red());
                println!("{} Run `oplire install` first", "Tip:".cyan().bold());
                return;
            }

            println!("{}", "Quick-resetting WARP tunnel...".bold());

            if cli.verbose {
                eprintln!("{} Disconnecting...", "[DEBUG]".cyan());
            }
            let _ = run_command("warp-cli", &["disconnect"], cli.dry_run, cli.verbose);

            if cli.verbose {
                eprintln!("{} Registration new...", "[DEBUG]".cyan());
            }
            let _ = run_command(
                "warp-cli",
                &["registration", "new"],
                cli.dry_run,
                cli.verbose,
            );

            if cli.verbose {
                eprintln!("{} Connecting...", "[DEBUG]".cyan());
            }
            let _ = run_command("warp-cli", &["connect"], cli.dry_run, cli.verbose);

            println!("{}", "Quick reset complete!".green().bold());
        }

        Commands::Config { action } => match action {
            ConfigAction::Show {} => {
                let config = load_config();
                println!("{}", "Current Configuration:".bold());
                println!();
                println!("{} {}", "Listen:".bold(), config.listen);
                println!("{} {}", "Upstream:".bold(), config.upstream);
                println!("{} {}", "Max Retries:".bold(), config.max_retries);
                println!("{} {}ms", "WARP Delay:".bold(), config.warp_delay);
                println!("{} {}", "Config file:".bold(), config_path().display());
            }
            ConfigAction::Set {
                key: _,
                listen,
                upstream,
                max_retries,
                warp_delay,
            } => {
                let mut config = load_config();

                config.listen = listen.clone();
                config.upstream = upstream.clone();
                config.max_retries = *max_retries;
                config.warp_delay = *warp_delay;

                match save_config(&config) {
                    Ok(()) => {
                        println!("{}", "Configuration saved!".green().bold());
                        println!("{} {}", "File:".bold(), config_path().display());
                    }
                    Err(e) => {
                        eprintln!("{} Failed to save config: {}", "[ERROR]".red(), e);
                        std::process::exit(1);
                    }
                }
            }
            ConfigAction::Reset {} => {
                let path = config_path();
                if path.exists() {
                    fs::remove_file(&path)
                        .map_err(|e| e.to_string())
                        .unwrap_or_else(|e| {
                            eprintln!("{} Failed to remove config: {}", "[ERROR]".red(), e);
                            std::process::exit(1);
                        });
                    println!("{}", "Configuration reset to defaults!".green().bold());
                } else {
                    println!("{} No configuration file found", "[INFO]".green());
                }
            }
        },

        Commands::Proxy {
            listen,
            upstream,
            api_key,
            max_retries,
            warp_delay,
        } => {
            let config = ProxyConfig {
                listen_addr: listen.clone(),
                opencode_base_url: upstream.clone(),
                opencode_api_key: api_key.clone(),
                max_retries: *max_retries,
                warp_reset_delay_ms: *warp_delay,
            };

            print_banner();
            println!("{}", "Anthropic ↔ OpenCode Zen Proxy".bold().cyan());
            println!();
            println!("{} Listening on: {}", "→".green(), listen.bold());
            println!("{} Upstream:     {}", "→".green(), upstream.bold());
            println!(
                "{} Max retries:  {}",
                "→".green(),
                max_retries.to_string().bold()
            );
            println!();
            println!(
                "{} Configure Claude Code to use: {}",
                "Tip:".cyan().bold(),
                format!("http://{}", listen).yellow().bold()
            );
            println!("{} Or run: {}", "→".cyan(), "oplire connect claude-code".bold());
            println!();

            let rt = tokio::runtime::Runtime::new().unwrap();
            if let Err(e) = rt.block_on(async {
                oplire_reset::proxy::start_proxy_server(config).await
            }) {
                eprintln!("{} Proxy server error: {}", "[ERROR]".red(), e);
                std::process::exit(1);
            }
        }

        Commands::Status { json } => {
            if !warp_installed {
                if *json {
                    println!("{{\"connected\": false, \"tunnel_id\": null, \"error\": \"warp-cli not installed\"}}");
                } else {
                    println!("{} {}", "Tunnel:".bold(), "Not connected".red());
                    println!("{} {}", "WARP:".bold(), "Not installed".red());
                    println!(
                        "\n{} Run `oplire install` to install WARP",
                        "Tip:".cyan().bold()
                    );
                }
                return;
            }

            println!("{}", "Checking tunnel status...".dimmed());

            match run_command("warp-cli", &["status"], cli.dry_run, cli.verbose) {
                Ok(output) => {
                    let status_str = output.trim();
                    let connected =
                        status_str.contains("Connected") || status_str.contains("connected");

                    if *json {
                        let tunnel_id = if connected { "active" } else { "null" };
                        println!(
                            "{{\"connected\": {}, \"tunnel_id\": \"{}\"}}",
                            connected, tunnel_id
                        );
                    } else {
                        if !status_str.is_empty() {
                            println!("{}", status_str);
                        }
                        let status = if connected {
                            "Active".green()
                        } else {
                            "Disconnected".red()
                        };
                        let tunnel = if connected {
                            "Connected".green()
                        } else {
                            "Not connected".red()
                        };
                        println!("\n{} {}", "Tunnel:".bold(), tunnel);
                        println!("{} {}", "WARP:".bold(), status);
                    }
                }
                Err(e) => {
                    if cli.verbose {
                        eprintln!("{} Error: {}", "[ERROR]".red(), e);
                    }
                    if *json {
                        println!(
                            "{{\"connected\": false, \"tunnel_id\": null, \"error\": \"{}\"}}",
                            e
                        );
                    } else {
                        println!("{} {}", "Tunnel:".bold(), "Error".red());
                        println!("{} {}", "Status:".bold(), e.red());
                    }
                }
            }
        }

        Commands::Reset {} => {
            if !warp_installed {
                println!("{} WARP is not installed", "[ERROR]".red());
                println!("{} Run `oplire install` first", "Tip:".cyan().bold());
                return;
            }

            println!("{}", "Resetting WARP tunnel...".bold());

            if cli.verbose {
                eprintln!("{} Step 1: Disconnecting...", "[DEBUG]".cyan());
            }
            let _ = run_command("warp-cli", &["disconnect"], cli.dry_run, cli.verbose);

            if cli.verbose {
                eprintln!("{} Step 2: Stopping warp-svc...", "[DEBUG]".cyan());
            }
            let _ = run_sudo_command("systemctl stop warp-svc", cli.dry_run, cli.verbose);

            if cli.verbose {
                eprintln!("{} Step 3: Clearing cache...", "[DEBUG]".cyan());
            }
            let _ = run_sudo_command(
                "rm -rf /var/lib/cloudflare-warp/*",
                cli.dry_run,
                cli.verbose,
            );

            if cli.verbose {
                eprintln!("{} Step 4: Starting warp-svc...", "[DEBUG]".cyan());
            }
            let _ = run_sudo_command("systemctl start warp-svc", cli.dry_run, cli.verbose);

            if cli.verbose {
                eprintln!("{} Step 5: Registering new tunnel...", "[DEBUG]".cyan());
            }
            let _ = run_command(
                "warp-cli",
                &["registration", "new"],
                cli.dry_run,
                cli.verbose,
            );

            if cli.verbose {
                eprintln!("{} Step 6: Connecting...", "[DEBUG]".cyan());
            }
            let _ = run_command("warp-cli", &["connect"], cli.dry_run, cli.verbose);

            println!("{}", "WARP tunnel reset complete!".green().bold());
            println!("{} Run `oplire status` to verify", "Tip:".cyan());
        }

        Commands::Install {} => {
            if warp_installed {
                println!("{} WARP is already installed", "[INFO]".green());
                return;
            }

            println!("{}", "Installing WARP tunnel...".bold());
            println!();
            println!("{}", "=== Linux Installation ===".bold());
            println!();
            println!("1. Add Cloudflare WARP repository:");
            println!("   curl -fsSL https://pkg.cloudflare.com/pubkey.gpg | sudo gpg --yes -o /usr/share/keyrings/cloudflare-warp-archive-keyring.gpg");
            println!("   echo 'deb [signed-by=/usr/share/keyrings/cloudflare-warp-archive-keyring.gpg] https://pkg.cloudflare.com/ any main' | sudo tee /etc/apt/sources.list.d/cloudflare-warp.list");
            println!();
            println!("2. Update and install:");
            println!("   sudo apt update");
            println!("   sudo apt install cloudflare-warp");
            println!();
            println!("3. Connect to WARP:");
            println!("   warp-cli connect");
            println!("   warp-cli status");
            println!();
            println!("{}", "=== macOS Installation ===".bold());
            println!();
            println!("1. Download the installer:");
            println!("   https://1111-releases.cloudflare.com/latest/mac/WARP.pkg");
            println!();
            println!("2. Run the installer package");
            println!();
            println!("3. Connect via terminal:");
            println!("   /Applications/Cloudflare\\ WARP.app/Contents/Resources/warp-cli connect");
            println!("   /Applications/Cloudflare\\ WARP.app/Contents/Resources/warp-cli status");
            println!();
            println!("{}", "=== Windows Installation ===".bold());
            println!();
            println!("1. Download the installer:");
            println!(
                "   https://1111-releases.cloudflare.com/latest/Windows Cloudflare WARP Setup.exe"
            );
            println!();
            println!("2. Run the installer as Administrator");
            println!();
            println!("3. Connect via PowerShell:");
            println!("   & 'C:\\Program Files (x86)\\Cloudflare\\Cloudflare WARP.exe' connect");
            println!();

            if cli.dry_run {
                println!(
                    "{} Installation instructions printed above",
                    "[DRY-RUN]".cyan()
                );
            } else {
                println!("{}", "Installation complete!".green().bold());
            }
        }

        Commands::Stop {} => {
            if !warp_installed {
                println!("{} WARP is not installed", "[ERROR]".red());
                println!("{} Run `oplire install` first", "Tip:".cyan().bold());
                return;
            }

            println!("{}", "Stopping WARP tunnel...".bold());

            if cli.verbose {
                eprintln!("{} Step 1: Disconnecting...", "[DEBUG]".cyan());
            }
            let _ = run_command("warp-cli", &["disconnect"], cli.dry_run, cli.verbose);

            if cli.verbose {
                eprintln!("{} Step 2: Stopping warp-svc...", "[DEBUG]".cyan());
            }
            let _ = run_sudo_command("systemctl stop warp-svc", cli.dry_run, cli.verbose);

            if cli.verbose {
                eprintln!("{} Step 3: Disabling warp-svc...", "[DEBUG]".cyan());
            }
            let _ = run_sudo_command("systemctl disable warp-svc", cli.dry_run, cli.verbose);

            println!("{}", "WARP tunnel stopped!".green().bold());
            println!("{} Run `oplire reset` to restart", "Tip:".cyan());
        }

        Commands::About {} => {
            print_banner();
            println!();
            println!("{} {}", "Version:".bold(), VERSION);
            println!("{} Rust", "Language:".bold());
            println!("{} OpenCode rate limit reset + Anthropic proxy", "Purpose:".bold());
            println!("{} Cloudflare WARP + Axum HTTP", "Infrastructure:".bold());
            println!("{} Berke Oruc", "Author:".bold());
            println!(
                "{} https://github.com/BerkeOruc/oplire",
                "GitHub:".bold()
            );
            println!();
            println!("{}", "Commands:".bold());
            println!("  oplire about               # Show this info");
            println!("  oplire status              # Check WARP status");
            println!("  oplire reset               # Full WARP tunnel reset");
            println!("  oplire quick-reset         # Fast WARP IP rotation");
            println!("  oplire stop                # Stop WARP tunnel");
            println!("  oplire install             # Install WARP");
            println!("  oplire proxy               # Start reverse proxy");
            println!("  oplire connect claude-code # Proxy + launch Claude Code");
            println!("  oplire watch               # Monitor OpenCode, auto-reset");
            println!("  oplire daemon              # Background proxy service");
            println!("  oplire config show         # Show current config");
            println!("  oplire config set          # Save config");
            println!("  oplire doctor              # Diagnose system setup");
        }
    }

    if !matches!(&cli.command, Commands::About {} | Commands::Proxy { .. } | Commands::Connect { .. } | Commands::Daemon { .. } | Commands::Watch { .. } | Commands::Doctor {} | Commands::Config { .. }) {
        println!("\n{} v{}", "oplire".bold(), VERSION.dimmed());
    }
}
