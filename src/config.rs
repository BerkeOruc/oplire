/// Proxy configuration for the Anthropic Messages bridge.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address to listen on (default: 127.0.0.1:8080)
    pub listen_addr: String,
    /// Upstream API root before /v1 (default: http://localhost:3000)
    pub opencode_base_url: String,
    /// API key for the upstream (if required)
    pub opencode_api_key: Option<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            opencode_base_url: "http://localhost:3000".to_string(),
            opencode_api_key: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub listen: String,
    pub upstream: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".to_string(),
            upstream: "http://localhost:3000".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_old_configs_with_warp_settings() {
        let config: AppConfig = serde_json::from_str(
            r#"{"upstream":"http://localhost:9000","max_retries":3,"warp_delay":5000}"#,
        )
        .unwrap();
        assert_eq!(config.upstream, "http://localhost:9000");
        assert_eq!(config.listen, AppConfig::default().listen);
    }
}
