use clap::{Parser, Subcommand};
use colored::Colorize;
use std::process::Command;

/// OpenCode Limit Reset - Cloudflare WARP tunnel reset tool
#[derive(Parser, Debug)]
#[command(name = "oplire-reset")]
#[command(version = "0.1.0")]
#[command(about = "Reset OpenCode limit by managing Cloudflare WARP tunnel", long_about = None)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Show what would be done without making changes
    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show current tunnel status and connection info
    ///
    /// Examples:
    ///   oplire-reset status
    ///   oplire-reset status --verbose
    Status {
        /// Show detailed JSON output
        #[arg(long)]
        json: bool,
    },

    /// Reset the WARP tunnel to refresh the connection
    ///
    /// Examples:
    ///   oplire-reset reset
    ///   oplire-reset reset --dry-run
    ///   oplire-reset reset --verbose
    Reset {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },

    /// Install or configure WARP tunnel
    ///
    /// Examples:
    ///   oplire-reset install
    ///   oplire-reset install --dry-run
    Install {
        /// Reinstall even if already installed
        #[arg(long)]
        force: bool,
    },

    /// Show information about oplire
    About {},
}

/// Check if warp-cli is installed
fn check_warp_installed() -> bool {
    Command::new("warp-cli").arg("--version").output().is_ok()
}

/// Run a command and return the output
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

/// Run a command with sudo
fn run_sudo_command(cmd: &str, dry_run: bool, verbose: bool) -> Result<String, String> {
    if dry_run {
        if verbose {
            eprintln!("{} Would execute: sudo {}", "[DRY-RUN]".cyan(), cmd);
        } else {
            eprintln!("{} sudo {}", "[DRY-RUN]".cyan(), cmd);
        }
        Ok(format!("[DRY-RUN] Would execute: sudo {}", cmd))
    } else {
        // Use sudo with -S flag to read password from stdin
        let output = Command::new("sudo")
            .arg("-S")
            .arg("-n") // non-interactive, will fail if no password
            .arg("-k") // reset sudo timestamp
            .arg("-v") // validate credentials
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

    // Check if warp-cli is installed for non-install commands
    let warp_installed = check_warp_installed();

    match &cli.command {
        Commands::Status { json } => {
            if !warp_installed {
                if *json {
                    println!("{{\"connected\": false, \"tunnel_id\": null, \"error\": \"warp-cli not installed\"}}");
                } else {
                    println!("{} {}", "Tunnel:".bold(), "Not connected".red());
                    println!("{} {}", "WARP:".bold(), "Not installed".red());
                    println!(
                        "\n{} Run `oplire-reset install` to install WARP",
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
        Commands::Reset { force } => {
            if !warp_installed {
                println!("{} WARP is not installed", "[ERROR]".red());
                println!("{} Run `oplire-reset install` first", "Tip:".cyan().bold());
                return;
            }

            if !*force && !cli.dry_run {
                println!(
                    "{} This will disconnect and reconnect your WARP tunnel",
                    "[WARNING]".yellow()
                );
                println!("{} Use --force to skip this confirmation", "Tip:".cyan());
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
            println!("{} Run `oplire-reset status` to verify", "Tip:".cyan());
        }
        Commands::Install { force } => {
            if warp_installed && !*force {
                println!("{} WARP is already installed", "[INFO]".green());
                println!("{} Use --force to reinstall", "Tip:".cyan());
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
        Commands::About {} => {
            println!(
                "{}",
                r#"
  _    ____  ___  ___  __  __  ___  ____ 
 / \  |  _ \|_ _|| _|/  \|  |/ _||  _ \
/ _ \ | | | || | | |_| | |\/| | | | |_) |
/ ___ \| |_| || | |  _  | |  | |_| |  _ /
/_/   \_\_/ |___||_| |_| |_|  |_|___) |
     OpenCode Limit Reset Tool              
     by Berke Oruc                     
"#
                .bold()
                .green()
            );
            println!();
            println!("{} {}", "Version:".bold(), "0.1.0");
            println!("{} {}", "Language:".bold(), "Rust");
            println!(
                "{} {}",
                "Purpose:".bold(),
                "OpenCode rate limit reset via Cloudflare WARP"
            );
            println!("{} {}", "Infrastructure:".bold(), "Cloudflare WARP Tunnel");
            println!(
                "{} {}",
                "GitHub:".bold(),
                "https://github.com/BerkeOruc/oplire"
            );
            println!();
            println!("{}", "Usage:".bold());
            println!("  oplire-reset status    # Check WARP status");
            println!("  oplire-reset reset     # Reset tunnel for new IP");
            println!("  oplire-reset install  # Install WARP");
        }
    }

    // Print version header
    println!("\n{} v{}", "oplire-reset".bold(), "0.1.0".dimmed());
}
