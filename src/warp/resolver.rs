use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{info, warn, error};

static ACTIVE_429_COUNT: AtomicU32 = AtomicU32::new(0);
static RESET_IN_PROGRESS: Mutex<()> = Mutex::const_new(());

pub struct WarpResolver {
    pub max_retries: u32,
    pub reset_delay_ms: u64,
}

impl WarpResolver {
    pub fn new(max_retries: u32, reset_delay_ms: u64) -> Self {
        Self {
            max_retries,
            reset_delay_ms,
        }
    }

    pub async fn handle_429(&self, retry_count: u32) -> bool {
        if retry_count >= self.max_retries {
            warn!(
                "Max retry attempts ({}) reached, giving up",
                self.max_retries
            );
            return false;
        }

        let _guard = match RESET_IN_PROGRESS.try_lock() {
            Ok(g) => g,
            Err(_) => {
                info!("WARP reset already in progress, waiting...");
                tokio::time::sleep(Duration::from_millis(self.reset_delay_ms)).await;
                return true;
            }
        };

        let count = ACTIVE_429_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        info!(
            "HTTP 429 detected (count: {}). Initiating WARP reset...",
            count
        );

        if let Err(e) = self.reset_warp().await {
            error!("WARP reset failed: {}", e);
            return false;
        }

        info!("WARP reset complete. Waiting for connection stabilization...");
        tokio::time::sleep(Duration::from_millis(self.reset_delay_ms)).await;

        ACTIVE_429_COUNT.fetch_sub(1, Ordering::SeqCst);
        true
    }

    async fn reset_warp(&self) -> Result<(), String> {
        info!("Step 1: warp-cli disconnect");
        run_warp_command(&["disconnect"]).await?;

        tokio::time::sleep(Duration::from_millis(1000)).await;

        info!("Step 2: systemctl stop warp-svc");
        run_sudo_command("systemctl stop warp-svc").await?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        info!("Step 3: Clearing WARP cache");
        run_sudo_command("rm -rf /var/lib/cloudflare-warp/*").await?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        info!("Step 4: systemctl start warp-svc");
        run_sudo_command("systemctl start warp-svc").await?;

        tokio::time::sleep(Duration::from_millis(2000)).await;

        info!("Step 5: warp-cli registration new");
        run_warp_command(&["registration", "new"]).await?;

        tokio::time::sleep(Duration::from_millis(1000)).await;

        info!("Step 6: warp-cli connect");
        run_warp_command(&["connect"]).await?;

        info!("WARP tunnel reset complete");
        Ok(())
    }
}

async fn run_warp_command(args: &[&str]) -> Result<(), String> {
    let output = Command::new("warp-cli")
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute warp-cli: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("warp-cli {:?} failed: {}", args, stderr.trim()))
    }
}

async fn run_sudo_command(cmd: &str) -> Result<(), String> {
    let output = Command::new("sudo")
        .arg("-n")
        .arg("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .await
        .map_err(|e| format!("Failed to execute sudo {}: {}", cmd, e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("sudo {} failed: {}", cmd, stderr.trim()))
    }
}
