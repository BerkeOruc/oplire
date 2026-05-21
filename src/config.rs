/// Proxy configuration for the Anthropic ↔ OpenCode Zen bridge.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address to listen on (default: 127.0.0.1:8080)
    pub listen_addr: String,
    /// OpenCode Zen API base URL (default: http://localhost:3000)
    pub opencode_base_url: String,
    /// API key for OpenCode Zen (if required)
    pub opencode_api_key: Option<String>,
    /// Maximum retry attempts after WARP reset
    pub max_retries: u32,
    /// Delay between WARP reset steps (milliseconds)
    pub warp_reset_delay_ms: u64,
}

/// A free model available on OpenCode Zen.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FreeModel {
    /// Model identifier used by OpenCode Zen
    pub id: String,
    /// Display name shown in Claude Code
    pub display_name: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            opencode_base_url: "http://localhost:3000".to_string(),
            opencode_api_key: None,
            max_retries: 3,
            warp_reset_delay_ms: 5000,
        }
    }
}

impl ProxyConfig {
    /// Returns the list of free models mapped to Anthropic schema.
    pub fn free_models() -> Vec<FreeModel> {
        vec![
            FreeModel {
                id: "glm-4.7-free".to_string(),
                display_name: "GLM 4.7 Free".to_string(),
            },
            FreeModel {
                id: "minimax-m2.1-free".to_string(),
                display_name: "MiniMax M2.1 Free".to_string(),
            },
            FreeModel {
                id: "kimi-k2.5-free".to_string(),
                display_name: "Kimi K2.5 Free".to_string(),
            },
            FreeModel {
                id: "qwen-2.5-72b-free".to_string(),
                display_name: "Qwen 2.5 72B Free".to_string(),
            },
            FreeModel {
                id: "llama-3.3-70b-free".to_string(),
                display_name: "Llama 3.3 70B Free".to_string(),
            },
        ]
    }

    /// Build Anthropic-compatible models response JSON.
    pub fn models_response() -> serde_json::Value {
        let models: Vec<serde_json::Value> = Self::free_models()
            .iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "object": "model",
                    "created": 0,
                    "owned_by": "opencode-zen"
                })
            })
            .collect();

        serde_json::json!({
            "object": "list",
            "data": models
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppConfig {
    pub listen: String,
    pub upstream: String,
    pub max_retries: u32,
    pub warp_delay: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".to_string(),
            upstream: "http://localhost:3000".to_string(),
            max_retries: 3,
            warp_delay: 5000,
        }
    }
}
