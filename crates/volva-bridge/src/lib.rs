#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeConfig {
    pub enabled: bool,
    pub server_url: String,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_url: "https://claude.ai".to_string(),
        }
    }
}

#[must_use]
pub fn bridge_status(config: &BridgeConfig) -> &'static str {
    if config.enabled {
        "experimental"
    } else {
        "disabled"
    }
}
