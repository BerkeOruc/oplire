use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::Serialize;
use std::{io::ErrorKind, process::Stdio, time::Duration};

#[derive(Clone, Copy, Debug, Subcommand)]
pub enum WarpAction {
    /// Check the local WARP client connection status
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Request a connection using the existing WARP registration
    Connect,
    /// Request a disconnection without changing the WARP registration
    Disconnect,
}

impl WarpAction {
    fn verb(self) -> &'static str {
        match self {
            Self::Status { .. } => "status",
            Self::Connect => "connect",
            Self::Disconnect => "disconnect",
        }
    }
}

pub fn setup_url() -> &'static str {
    match std::env::consts::OS {
        "linux" => "https://developers.cloudflare.com/warp-client/get-started/linux/",
        "macos" => "https://developers.cloudflare.com/warp-client/get-started/macos/",
        "windows" => "https://developers.cloudflare.com/warp-client/get-started/windows/",
        _ => "https://developers.cloudflare.com/warp-client/get-started/",
    }
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Connected,
    Disconnected,
    Connecting,
    Unknown,
}

fn parse_status(output: &str) -> State {
    let status = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Status update:").map(str::trim));
    match status {
        Some("Connected") => State::Connected,
        Some("Disconnected") => State::Disconnected,
        Some("Connecting") => State::Connecting,
        _ => State::Unknown,
    }
}

fn status_json(state: State, error: Option<&str>) -> serde_json::Value {
    let mut status = serde_json::json!({"connected": state == State::Connected, "state": state});
    if let Some(message) = error {
        status["error"] = serde_json::Value::String(message.into());
    }
    status
}

async fn execute(action: WarpAction) -> Result<String> {
    let verb = action.verb();
    let mut command = tokio::process::Command::new("warp-cli");
    command.arg(verb).stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .with_context(|| format!("warp-cli {verb} timed out"))?
        .map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                anyhow::anyhow!("warp-cli was not found on PATH. See {}", setup_url())
            } else {
                anyhow::Error::new(error).context(format!("could not run warp-cli {verb}"))
            }
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        bail!("warp-cli {verb} failed ({}): {detail}", output.status);
    }
    Ok(stdout.trim().into())
}

pub fn run(action: WarpAction, dry_run: bool, verbose: bool) -> bool {
    let verb = action.verb();
    if dry_run && !matches!(action, WarpAction::Status { .. }) {
        eprintln!("[DRY-RUN] warp-cli {verb}");
        return true;
    }
    if verbose {
        eprintln!("Running warp-cli {verb}");
    }
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("could not start WARP command runtime")
        .and_then(|runtime| runtime.block_on(execute(action)));
    let json = matches!(action, WarpAction::Status { json: true });
    match result {
        Ok(output) => {
            if json {
                println!("{}", status_json(parse_status(&output), None));
            } else if !output.is_empty() {
                println!("{output}");
            } else if matches!(action, WarpAction::Status { .. }) {
                println!("WARP status unavailable");
            }
            true
        }
        Err(error) => {
            let message = format!("{error:#}");
            if json {
                println!("{}", status_json(State::Unknown, Some(&message)));
            } else {
                eprintln!("{message}");
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_the_status_line() {
        for (output, expected) in [
            ("Status update: Connected\r\n", State::Connected),
            (
                "Status update: Disconnected\nReason: Connected earlier",
                State::Disconnected,
            ),
            ("Status update: Connecting", State::Connecting),
            ("Reason: Connected", State::Unknown),
            ("Status update: Unexpected", State::Unknown),
            ("", State::Unknown),
        ] {
            assert_eq!(parse_status(output), expected);
        }
    }

    #[test]
    fn serializes_status_and_error_without_fabricating_a_tunnel_id() {
        let message = "Client said \"not ready\"\nTry again";
        let output = status_json(State::Unknown, Some(message)).to_string();
        let decoded: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(decoded["connected"], false);
        assert_eq!(decoded["state"], "unknown");
        assert_eq!(decoded["error"], message);
        assert_eq!(status_json(State::Connected, None)["connected"], true);
    }
}
