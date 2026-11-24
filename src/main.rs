use clap::{Parser, Subcommand};
use colored::Colorize;
use oplire_reset::{ProxyConfig, AppConfig};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

mod warp;
use warp::WarpAction;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(name = "oplire")]
#[command(version = VERSION)]
#[command(about = "Anthropic Messages bridge with optional Cloudflare WARP controls", long_about = None)]
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
    #[command(hide = true)]
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Control the installed Cloudflare WARP client
    Warp {
        #[command(subcommand)]
        action: WarpAction,
    },
    Install {
        #[command(subcommand)]
        target: InstallTarget,
    },
    #[command(hide = true)]
    Stop {},
    About {},
    Proxy {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
        #[arg(long)]
        api_key: Option<String>,
    },
    Connect {
        #[command(subcommand)]
        target: ConnectTarget,
    },
    Daemon {
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
        #[arg(long)]
        api_key: Option<String>,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Doctor {},
    Setup {},
    Models {
        #[arg(long, default_value = "http://localhost:3000")]
        upstream: String,
    },
}

#[derive(Subcommand, Debug)]
enum InstallTarget {
    /// Show official Cloudflare WARP installation instructions
    Warp {},
    /// Install the OpenCode V2 CLI with Bun
    Opencode {},
    /// Install Claude Code CLI
    ClaudeCode {},
    /// Install OpenCode and Claude Code
    All {},
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
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
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

fn check_claude_installed() -> bool {
    Command::new("claude").arg("--version").output().is_ok()
}

fn check_opencode_installed() -> bool {
    Command::new("opencode").arg("--version").output().is_ok()
        || Command::new("opencode").arg("--help").output().is_ok()
}

fn check_upstream_models(base_url: &str) -> bool {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    reqwest::blocking::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .and_then(|r| r.error_for_status()?.json::<serde_json::Value>())
        .map(|body| body.get("data").is_some_and(serde_json::Value::is_array))
        .unwrap_or(false)
}

fn check_node_installed() -> bool {
    Command::new("node").arg("--version").output().is_ok()
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

fn run_interactive(cmd: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {}", cmd, status))
    }
}

fn fetch_models(upstream: &str, api_key: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let models_url = format!("{}/v1/models", upstream.trim_end_matches('/'));

    let mut request = reqwest::blocking::Client::new()
        .get(&models_url)
        .timeout(std::time::Duration::from_secs(10));
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }
    let resp = request.send().map_err(|e| e.without_url().to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let data = resp.json::<serde_json::Value>().map_err(|e| e.to_string())?;

    let models = data
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "No models in response".to_string())?;

    Ok(models
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?.to_string();
            let name = match m.get("name")
                .or_else(|| m.get("display_name"))
                .and_then(|v| v.as_str())
            {
                Some(n) => n.to_string(),
                None => id.replace('-', " ")
                    .split_whitespace()
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(ch) => format!(
                                "{}{}",
                                ch.to_uppercase().collect::<String>(),
                                c.as_str()
                            ),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            Some((id, name))
        })
        .collect())
}

fn select_model_interactively(models: &[(String, String)]) -> Option<String> {
    use std::io::{self, Write};

    if models.is_empty() {
        return None;
    }

    println!();
    println!("{}", "Available models:".bold());
    println!();

    for (i, (id, name)) in models.iter().enumerate() {
        println!("  {} {} — {}", format!("[{}]", i + 1).dimmed(), id.bold().yellow(), name);
    }

    println!();
    print!("{} Select model (1-{}): ", "→".cyan(), models.len());
    io::stdout().flush().ok()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok()?;
    let num: usize = input.trim().parse().ok()?;

    if num >= 1 && num <= models.len() {
        Some(models[num - 1].0.clone())
    } else {
        None
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

fn print_step(num: usize, text: &str) {
    println!("  {} {}", format!("[{}/5]", num).dimmed(), text.bold());
}

fn print_success(text: &str) {
    println!("  {} {}", "✓".green().bold(), text);
}

fn print_fail(text: &str) {
    println!("  {} {}", "✗".red().bold(), text);
}

fn print_info_formatted(label: &str, desc: &str) {
    println!("  {} {} — {}", "→".cyan(), label.bold(), desc.dimmed());
}

fn run_warp(action: WarpAction, cli: &Cli) {
    if !warp::run(action, cli.dry_run, cli.verbose) {
        std::process::exit(1);
    }
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

    match &cli.command {
        Commands::Models { upstream } => {
            print_banner();
            println!("{}", "Available Models".bold().cyan());
            println!();
            println!("{} Fetching models from {}...", "→".cyan(), upstream.bold());
            println!();

            let models_url = format!("{}/v1/models", upstream.trim_end_matches('/'));

            match reqwest::blocking::Client::new()
                .get(&models_url)
                .timeout(std::time::Duration::from_secs(10))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    match resp.json::<serde_json::Value>() {
                        Ok(data) => {
                            if let Some(models) = data.get("data").and_then(|v| v.as_array()) {
                                if models.is_empty() {
                                    println!("{} No models found", "[INFO]".yellow());
                                    return;
                                }

                                println!("  {}  {:<30}  {}", "#".dimmed(), "ID".bold(), "Name".bold());
                                println!("  {}", "─".repeat(60).dimmed());

                                for (i, m) in models.iter().enumerate() {
                                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                                    let name: String = match m.get("name")
                                        .or_else(|| m.get("display_name"))
                                        .and_then(|v| v.as_str())
                                    {
                                        Some(n) => n.to_string(),
                                        None => id.replace('-', " ").split_whitespace()
                                            .map(|w| {
                                                let mut c = w.chars();
                                                match c.next() {
                                                    None => String::new(),
                                                    Some(ch) => format!("{}{}", ch.to_uppercase().collect::<String>(), c.as_str()),
                                                }
                                            })
                                            .collect::<Vec<_>>().join(" "),
                                    };

                                    println!("  {}  {:<30}  {}",
                                        format!("[{}]", i + 1).dimmed(),
                                        id.yellow(),
                                        name
                                    );
                                }

                                println!();
                                println!("{}", "Usage:".bold());
                                println!("  {}", "oplire connect claude-code --model <id>".bold().yellow());
                                println!();
                                println!("{}", "Examples:".bold());
                                for m in models.iter().take(3) {
                                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    if !id.is_empty() {
                                        println!("  {}", format!("oplire connect claude-code --model {}", id).dimmed());
                                    }
                                }
                            } else {
                                println!("{} No 'data' field in response", "[ERROR]".red());
                            }
                        }
                        Err(e) => {
                            println!("{} Failed to parse response: {}", "[ERROR]".red(), e);
                        }
                    }
                }
                Ok(resp) => {
                    println!("{} Upstream returned status: {}", "[ERROR]".red(), resp.status());
                    println!("{} Check the inference API at {}.", "Tip:".cyan(), upstream.bold());
                }
                Err(e) => {
                    println!("{} Failed to connect to {}", "[ERROR]".red(), upstream.bold());
                    println!("{} {}", "Error:".dimmed(), e.to_string().dimmed());
                    println!("{} Check that the inference API is running.", "Tip:".cyan());
                }
            }
        }

        Commands::Setup {} => {
            print_banner();
            println!("{}", "Welcome to oplire Setup Wizard".bold().green());
            println!();
            println!("{}", "This will check and install all required components.".dimmed());
            println!();

            let mut steps_needed = Vec::new();

            if !check_node_installed() {
                steps_needed.push(("Node.js", "https://nodejs.org/", "Required for the Claude Code npm installer"));
            }
            if !check_opencode_installed() {
                steps_needed.push(("OpenCode", "oplire install opencode", "AI coding assistant backend"));
            }
            if !check_claude_installed() {
                steps_needed.push(("Claude Code", "oplire install claudecode", "AI coding assistant CLI"));
            }

            if steps_needed.is_empty() {
                println!("{}", "Everything is already installed!".green().bold());
                println!();
                println!("{} Run to get started:", "Tip:".cyan().bold());
                println!("  {}", "oplire connect claude-code".bold().yellow());
                return;
            }

            println!("{}", "The following components need to be installed:".bold());
            println!();
            for (name, _, desc) in &steps_needed {
                print_info_formatted(name, desc);
            }
            println!();

            for (i, (name, _cmd, _)) in steps_needed.iter().enumerate() {
                println!();
                print_step(i + 1, &format!("Installing {}...", name));

                match *name {
                    "Node.js" => {
                        println!("  {} Node.js must be installed manually", "ℹ".yellow());
                        println!("  {}", "curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash".dimmed());
                        println!("  {}", "Then restart your terminal".dimmed());
                    }
                    "OpenCode" => {
                        let _ = run_interactive("oplire", &["install", "opencode"]);
                    }
                    "Claude Code" => {
                        let _ = run_interactive("oplire", &["install", "claude-code"]);
                    }
                    _ => {}
                }
            }

            println!();
            println!("{}", "Setup complete!".green().bold());
                println!();
                println!("{}", "Next steps:".bold());
                println!("  1. {}", "oplire doctor".bold().yellow());
                println!("  2. {}", "oplire connect claude-code".bold().yellow());
        }

        Commands::Install { target } => match target {
            InstallTarget::Warp {} => {
                println!("Set up Cloudflare WARP: {}", warp::setup_url());
            }

            InstallTarget::Opencode {} => {
                print_banner();
                println!("{}", "Installing OpenCode".bold().green());
                println!();

                if check_opencode_installed() {
                    println!("{} OpenCode is already installed", "[INFO]".green());
                    println!("V1 to V2 migration: https://opencode.ai/docs/migrate-v1");
                    return;
                }

                let args = ["install", "-g", "--trust", "@opencode/cli"];
                if cli.dry_run {
                    let _ = run_command("bun", &args, true, cli.verbose);
                    return;
                }

                if !Command::new("bun")
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
                {
                    print_fail("Bun is required for this installer. See https://opencode.ai/docs/#cli for other options.");
                    std::process::exit(1);
                }

                print_step(1, "Installing OpenCode V2 via Bun...");
                match run_interactive("bun", &args) {
                    Ok(()) => {
                        print_success("OpenCode installed");
                        println!();
                        println!("  Run `opencode` and use `/models` to select a model.");
                    }
                    Err(e) => {
                        print_fail(&format!("Installation failed: {}", e));
                        println!();
                        println!("{} Try manually:", "Fix:".cyan());
                        println!("  {}", "bun install -g --trust @opencode/cli".bold().yellow());
                        std::process::exit(1);
                    }
                }
            }

            InstallTarget::ClaudeCode {} => {
                print_banner();
                println!("{}", "Installing Claude Code".bold().green());
                println!();

                if check_claude_installed() {
                    let version = run_command("claude", &["--version"], false, false)
                        .map(|v| v.trim().to_string())
                        .unwrap_or_else(|_| "unknown".to_string());
                    println!("{} Claude Code is already installed ({})", "[INFO]".green(), version.dimmed());
                    return;
                }

                if !check_node_installed() {
                    println!("{} Node.js is required but not found", "[ERROR]".red());
                    println!("{} Install Node.js first: https://nodejs.org/", "Fix:".cyan());
                    std::process::exit(1);
                }

                print_step(1, "Installing Claude Code via npm...");
                match run_interactive("npm", &["install", "-g", "@anthropic-ai/claude-code"]) {
                    Ok(()) => {
                        print_success("Claude Code installed");
                        println!();
                        println!("{}", "Next steps:".bold());
                        println!("  1. {}", "claude --version".bold().yellow());
                        println!("  2. {}", "oplire connect claude-code".bold().yellow());
                    }
                    Err(e) => {
                        print_fail(&format!("Installation failed: {}", e));
                        println!();
                        println!("{} Try manually:", "Fix:".cyan());
                        println!("  {}", "npm install -g @anthropic-ai/claude-code".bold().yellow());
                    }
                }
            }

            InstallTarget::All {} => {
                print_banner();
                println!("{}", "Installing All Components".bold().green());
                println!();

                print_step(1, "Checking OpenCode...");
                if check_opencode_installed() {
                    print_success("OpenCode already installed");
                } else {
                    let _ = run_interactive("oplire", &["install", "opencode"]);
                }

                print_step(2, "Checking Claude Code...");
                if check_claude_installed() {
                    print_success("Claude Code already installed");
                } else {
                    let _ = run_interactive("oplire", &["install", "claude-code"]);
                }

                println!();
                println!("{}", "All components installed!".green().bold());
                println!();
                println!("{}", "Run to get started:".bold());
                println!("  {}", "oplire connect claude-code".bold().yellow());
            }
        },

        Commands::Connect {
            target: ConnectTarget::ClaudeCode {
                listen,
                upstream,
                api_key,
                model,
                system_prompt,
                claude_args,
            },
        } => {
            print_banner();
            println!("{}", "Claude Code Bridge".bold().green());
            println!();

            if !check_claude_installed() {
                eprintln!("{} Claude Code not found in PATH", "[ERROR]".red());
                eprintln!("{} Install: {}", "Fix:".cyan(), "oplire install claude-code".bold().yellow());
                std::process::exit(1);
            }

            let selected_model = match model {
                Some(m) => m.clone(),
                None => {
                    println!("{} Fetching models from {}...", "→".cyan(), upstream.bold());
                    fetch_models(upstream, api_key.as_deref())
                        .map_err(|error| eprintln!("Could not fetch upstream models: {error}"))
                        .ok()
                        .and_then(|models| select_model_interactively(&models))
                        .unwrap_or_default()
                }
            };
            if selected_model.trim().is_empty() {
                eprintln!("Select an upstream model or pass --model <id>");
                std::process::exit(1);
            }

            let config = ProxyConfig {
                listen_addr: listen.clone(),
                opencode_base_url: upstream.clone(),
                opencode_api_key: api_key.clone(),
            };

            let rt = tokio::runtime::Runtime::new().expect("could not start proxy runtime");
            let (listener, app) = rt
                .block_on(oplire_reset::proxy::server::bind_proxy(&config))
                .unwrap_or_else(|error| {
                    eprintln!("Could not start the bridge on {listen}: {error}");
                    std::process::exit(1);
                });
            let address = listener.local_addr().expect("bound proxy address");
            rt.spawn(async move {
                if let Err(error) = axum::serve(listener, app).await {
                    eprintln!("Bridge stopped: {error}");
                }
            });

            println!("{} Proxy:      {}", "→".green(), address.to_string().bold());
            println!("{} Upstream:   {}", "→".green(), upstream.bold());
            println!("{} Model:     {}", "→".green(), selected_model.bold());
            if let Some(sp) = system_prompt {
                println!("{} System:    {} chars", "→".green(), sp.len().to_string().bold());
            }
            println!();

            println!("{}", "Launching Claude Code...".dimmed());
            println!();

            let mut cmd = Command::new("claude");
            cmd.env("ANTHROPIC_BASE_URL", format!("http://{address}"))
                .env("ANTHROPIC_AUTH_TOKEN", "oplire-local")
                .env_remove("ANTHROPIC_API_KEY")
                .env("ANTHROPIC_MODEL", &selected_model);
            for name in [
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
                "CLAUDE_CODE_USE_MANTLE",
                "CLAUDE_CODE_USE_ANTHROPIC_AWS",
                "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
            ] {
                cmd.env_remove(name);
            }
            for name in [
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                "ANTHROPIC_DEFAULT_OPUS_MODEL",
            ] {
                cmd.env(
                    name,
                    std::env::var_os(name).unwrap_or_else(|| selected_model.as_str().into()),
                );
            }
            for (name, value) in [
                ("CLAUDE_CODE_DISABLE_THINKING", "1"),
                ("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS", "1"),
                ("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "1"),
                ("CLAUDE_CODE_ATTRIBUTION_HEADER", "0"),
            ] {
                cmd.env(name, std::env::var_os(name).unwrap_or_else(|| value.into()));
            }

            if let Some(sp) = system_prompt {
                cmd.args(["--append-system-prompt", sp]);
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

            drop(rt);
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }

        Commands::Daemon {
            listen,
            upstream,
            api_key,
        } => {
            print_banner();
            println!("{}", "Daemon Mode".bold().magenta());
            println!();
            println!("{} Listening:  {}", "→".green(), listen.bold());
            println!("{} Upstream:   {}", "→".green(), upstream.bold());
            println!();

            let config = ProxyConfig {
                listen_addr: listen.clone(),
                opencode_base_url: upstream.clone(),
                opencode_api_key: api_key.clone(),
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
            let mut checks = Vec::new();

            if check_claude_installed() {
                let version = run_command("claude", &["--version"], false, false)
                    .map(|v| v.trim().to_string())
                    .unwrap_or_else(|_| "unknown".to_string());
                checks.push(("Claude Code", format!("installed ({})", version), true));
            } else {
                checks.push(("Claude Code", "NOT FOUND".to_string(), false));
                all_ok = false;
            }

            if check_opencode_installed() {
                checks.push(("OpenCode", "installed".to_string(), true));
            } else {
                checks.push(("OpenCode", "NOT FOUND".to_string(), false));
                all_ok = false;
            }

            let config = load_config();
            let upstream = config.upstream.clone();
            if check_upstream_models(&upstream) {
                checks.push(("Upstream models", format!("available ({})", upstream), true));
            } else {
                checks.push(("Upstream models", format!("UNAVAILABLE ({})", upstream), false));
                all_ok = false;
            }

            let cfg_path = config_path();
            if cfg_path.exists() {
                checks.push(("Config file", format!("found ({})", cfg_path.display()), true));
            } else {
                checks.push(("Config file", "using defaults".to_string(), true));
            }

            if std::net::TcpListener::bind("127.0.0.1:8080").is_ok() {
                checks.push(("Port 8080", "available".to_string(), true));
            } else {
                checks.push(("Port 8080", "IN USE".to_string(), false));
                all_ok = false;
            }

            for (name, status, ok) in &checks {
                let status_str = if *ok {
                    status.green().bold()
                } else {
                    status.red().bold()
                };
                println!("{} {}: {}", "→".cyan(), name.bold(), status_str);
            }

            println!();
            if all_ok {
                println!("{}", "All checks passed!".green().bold());
            } else {
                println!("{}", "Some checks failed.".yellow().bold());
                println!();
                println!("{} Run `oplire setup` for guided installation", "Fix:".cyan());
            }
        }

        Commands::Config { action } => match action {
            ConfigAction::Show {} => {
                let config = load_config();
                println!("{}", "Current Configuration:".bold());
                println!();
                println!("{} {}", "Listen:".bold(), config.listen);
                println!("{} {}", "Upstream:".bold(), config.upstream);
                println!("{} {}", "Config file:".bold(), config_path().display());
            }
            ConfigAction::Set {
                key: _,
                listen,
                upstream,
            } => {
                let mut config = load_config();

                config.listen = listen.clone();
                config.upstream = upstream.clone();

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
        } => {
            let config = ProxyConfig {
                listen_addr: listen.clone(),
                opencode_base_url: upstream.clone(),
                opencode_api_key: api_key.clone(),
            };

            print_banner();
            println!("{}", "Anthropic Messages Proxy".bold().cyan());
            println!();
            println!("{} Listening on: {}", "→".green(), listen.bold());
            println!("{} Upstream:     {}", "→".green(), upstream.bold());
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

        Commands::Warp { action } => run_warp(*action, &cli),
        Commands::Status { json } => run_warp(WarpAction::Status { json: *json }, &cli),
        Commands::Stop {} => run_warp(WarpAction::Disconnect, &cli),

        Commands::About {} => {
            print_banner();
            println!();
            println!("{} {}", "Version:".bold(), VERSION);
            println!("{} Rust", "Language:".bold());
            println!("{} Anthropic Messages bridge", "Purpose:".bold());
            println!("{} Cloudflare WARP + Axum HTTP", "Infrastructure:".bold());
            println!("{} Berke Oruc", "Author:".bold());
            println!(
                "{} https://github.com/BerkeOruc/oplire",
                "GitHub:".bold()
            );
            println!();
            println!("{}", "Installation:".bold());
            println!("  oplire install warp          # Show Cloudflare WARP setup instructions");
            println!("  oplire install opencode      # Install OpenCode");
            println!("  oplire install claude-code   # Install Claude Code CLI");
            println!("  oplire install all           # Install everything");
            println!("  oplire setup                 # Guided setup wizard");
            println!();
            println!("{}", "WARP Management:".bold());
            println!("  oplire warp status         # Check WARP status");
            println!("  oplire warp connect        # Request a WARP connection");
            println!("  oplire warp disconnect     # Request a WARP disconnection");
            println!();
            println!("{}", "Proxy & Claude Code:".bold());
            println!("  oplire proxy               # Start reverse proxy");
            println!("  oplire connect claude-code # Proxy + launch Claude Code");
            println!("  oplire daemon              # Run the proxy in the foreground");
            println!();
            println!("{}", "Configuration:".bold());
            println!("  oplire models              # List available OpenCode models");
            println!("  oplire config show         # Show current config");
            println!("  oplire config set          # Save config");
            println!("  oplire config reset        # Reset to defaults");
            println!("  oplire doctor              # Diagnose system setup");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn warp_commands_are_explicit_and_legacy_reset_options_are_rejected() {
        Cli::command().debug_assert();
        let status = Cli::try_parse_from(["oplire", "warp", "status", "--json"]).unwrap();
        assert!(matches!(
            status.command,
            Commands::Warp { action: WarpAction::Status { json: true } }
        ));
        for command in ["reset", "quick-reset", "watch"] {
            assert!(Cli::try_parse_from(["oplire", command]).is_err());
        }
        assert!(Cli::try_parse_from(["oplire", "proxy", "--warp-delay", "1"]).is_err());
    }
}
