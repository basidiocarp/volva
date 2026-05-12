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

#[cfg(test)]
mod tests {
    use super::{BridgeConfig, bridge_status};

    #[test]
    fn default_is_disabled_pointing_at_claude_ai() {
        let cfg = BridgeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.server_url, "https://claude.ai");
    }

    #[test]
    fn status_disabled_when_not_enabled() {
        let cfg = BridgeConfig::default();
        assert_eq!(bridge_status(&cfg), "disabled");
    }

    #[test]
    fn status_experimental_when_enabled() {
        let cfg = BridgeConfig {
            enabled: true,
            server_url: "https://claude.ai".to_string(),
        };
        assert_eq!(bridge_status(&cfg), "experimental");
    }

    #[test]
    fn custom_server_url_preserved() {
        let cfg = BridgeConfig {
            enabled: false,
            server_url: "https://internal.example.com".to_string(),
        };
        assert_eq!(cfg.server_url, "https://internal.example.com");
    }

    #[test]
    fn clone_produces_equal_value() {
        let cfg = BridgeConfig::default();
        assert_eq!(cfg.clone(), cfg);
    }
}
