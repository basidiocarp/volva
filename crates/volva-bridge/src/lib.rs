//! Bridge configuration.
//!
//! This crate provides basic bridge configuration (enabled/disabled state and server URL).
//! The enabled state is loaded from `VolvaConfig.experimental_bridge` at runtime.
//! Additional fields (schema support, profile loading) may be added in future versions.

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
