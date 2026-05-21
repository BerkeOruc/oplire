use std::time::Duration;
use tokio::time::interval;
use tracing::{info, warn, error};

use crate::warp::WarpResolver;

pub async fn start_watch_mode(
    upstream: &str,
    max_retries: u32,
    warp_delay_ms: u64,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let resolver = WarpResolver::new(max_retries, warp_delay_ms);
    let models_url = format!("{}/v1/models", upstream.trim_end_matches('/'));

    let mut check_interval = interval(Duration::from_secs(30));
    let mut consecutive_429 = 0u32;

    info!("Watch mode active. Checking {} every 30s", models_url);

    loop {
        check_interval.tick().await;

        match client.get(&models_url).send().await {
            Ok(resp) => {
                let status = resp.status();

                if status.is_success() {
                    if consecutive_429 > 0 {
                        info!("Rate limit recovered (was {} consecutive 429s)", consecutive_429);
                    }
                    consecutive_429 = 0;
                    info!("OpenCode Zen: OK (status {})", status);
                } else if status.as_u16() == 429 {
                    consecutive_429 += 1;
                    warn!(
                        "Rate limit detected (count: {}). Triggering WARP reset...",
                        consecutive_429
                    );

                    if resolver.handle_429(consecutive_429 - 1).await {
                        info!("WARP reset triggered. Rechecking...");
                        tokio::time::sleep(Duration::from_secs(5)).await;

                        match client.get(&models_url).send().await {
                            Ok(retry_resp) if retry_resp.status().is_success() => {
                                info!("Recovery confirmed. OpenCode Zen is accessible.");
                                consecutive_429 = 0;
                            }
                            _ => {
                                warn!("Recovery check failed. Will retry on next cycle.");
                            }
                        }
                    } else {
                        error!("WARP reset failed. Rate limit persists.");
                    }
                } else {
                    warn!("OpenCode Zen returned unexpected status: {}", status);
                }
            }
            Err(e) => {
                error!("Failed to reach OpenCode Zen: {}", e);
            }
        }
    }
}
