use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use volva_core::BackendKind;

fn default_vendor_dir() -> PathBuf {
    PathBuf::from("vendor")
}

fn default_backend_command() -> String {
    "claude".to_string()
}

fn default_hook_adapter_enabled() -> bool {
    false
}

const fn default_hook_adapter_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendConfig {
    #[serde(default = "default_backend_kind")]
    pub kind: BackendKind,
    #[serde(default = "default_backend_command")]
    pub command: String,
}

const fn default_backend_kind() -> BackendKind {
    BackendKind::OfficialCli
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            kind: default_backend_kind(),
            command: default_backend_command(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookAdapterConfig {
    #[serde(default = "default_hook_adapter_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_hook_adapter_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for HookAdapterConfig {
    fn default() -> Self {
        Self {
            enabled: default_hook_adapter_enabled(),
            command: None,
            args: Vec::new(),
            timeout_ms: default_hook_adapter_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolvaConfig {
    pub model: String,
    pub api_base_url: String,
    pub experimental_bridge: bool,
    #[serde(default)]
    pub backend: BackendConfig,
    #[serde(default)]
    pub hook_adapter: HookAdapterConfig,
    #[serde(default = "default_vendor_dir")]
    pub vendor_dir: PathBuf,
}

impl Default for VolvaConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".to_string(),
            api_base_url: "https://api.anthropic.com".to_string(),
            experimental_bridge: false,
            backend: BackendConfig::default(),
            hook_adapter: HookAdapterConfig::default(),
            vendor_dir: default_vendor_dir(),
        }
    }
}

impl VolvaConfig {
    pub fn load_from(root: &Path) -> Result<Self> {
        let path = root.join("volva.json");
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path)?;
        let mut config = serde_json::from_str::<Self>(&raw)?;
        if config.vendor_dir.is_relative() {
            config.vendor_dir = root.join(&config.vendor_dir);
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendConfig, HookAdapterConfig, VolvaConfig};
    use volva_core::BackendKind;

    #[test]
    fn default_config_uses_official_cli_backend() {
        let config = VolvaConfig::default();

        assert_eq!(
            config.backend,
            BackendConfig {
                kind: BackendKind::OfficialCli,
                command: "claude".to_string(),
            }
        );
        assert_eq!(config.hook_adapter, HookAdapterConfig::default());
    }

    #[test]
    fn backend_defaults_when_missing_from_json() {
        let config = serde_json::from_str::<VolvaConfig>(
            r#"{
              "model": "claude-sonnet-4-6",
              "api_base_url": "https://api.anthropic.com",
              "experimental_bridge": false
            }"#,
        )
        .expect("config should deserialize");

        assert_eq!(config.backend.kind, BackendKind::OfficialCli);
        assert_eq!(config.backend.command, "claude");
        assert_eq!(config.hook_adapter, HookAdapterConfig::default());
    }

    #[test]
    fn hook_adapter_defaults_when_missing_from_json() {
        let config = serde_json::from_str::<VolvaConfig>(
            r#"{
              "model": "claude-sonnet-4-6",
              "api_base_url": "https://api.anthropic.com",
              "experimental_bridge": false,
              "backend": {
                "kind": "official-cli",
                "command": "claude"
              }
            }"#,
        )
        .expect("config should deserialize");

        assert_eq!(config.hook_adapter, HookAdapterConfig::default());
    }

    #[test]
    fn hook_adapter_deserializes_enabled_and_command() {
        let config = serde_json::from_str::<VolvaConfig>(
            r#"{
              "model": "claude-sonnet-4-6",
              "api_base_url": "https://api.anthropic.com",
              "experimental_bridge": false,
              "backend": {
                "kind": "official-cli",
                "command": "claude"
              },
              "hook_adapter": {
                "enabled": true,
                "command": "/usr/local/bin/cortina-hook-adapter"
              }
            }"#,
        )
        .expect("config should deserialize");

        assert!(config.hook_adapter.enabled);
        assert_eq!(
            config.hook_adapter.command.as_deref(),
            Some("/usr/local/bin/cortina-hook-adapter")
        );
        assert!(config.hook_adapter.args.is_empty());
        assert_eq!(config.hook_adapter.timeout_ms, 30_000);
    }

    #[test]
    fn hook_adapter_deserializes_command_and_args() {
        let config = serde_json::from_str::<VolvaConfig>(
            r#"{
              "model": "claude-sonnet-4-6",
              "api_base_url": "https://api.anthropic.com",
              "experimental_bridge": false,
              "backend": {
                "kind": "official-cli",
                "command": "claude"
              },
              "hook_adapter": {
                "enabled": true,
                "command": "cortina",
                "args": ["adapter", "volva", "hook-event"]
              }
            }"#,
        )
        .expect("config should deserialize");

        assert!(config.hook_adapter.enabled);
        assert_eq!(config.hook_adapter.command.as_deref(), Some("cortina"));
        assert_eq!(
            config.hook_adapter.args,
            vec![
                "adapter".to_string(),
                "volva".to_string(),
                "hook-event".to_string()
            ]
        );
        assert_eq!(config.hook_adapter.timeout_ms, 30_000);
    }

    #[test]
    fn hook_adapter_deserializes_timeout_ms() {
        let config = serde_json::from_str::<VolvaConfig>(
            r#"{
              "model": "claude-sonnet-4-6",
              "api_base_url": "https://api.anthropic.com",
              "experimental_bridge": false,
              "backend": {
                "kind": "official-cli",
                "command": "claude"
              },
              "hook_adapter": {
                "enabled": true,
                "command": "cortina",
                "args": ["adapter", "volva", "hook-event"],
                "timeout_ms": 45000
              }
            }"#,
        )
        .expect("config should deserialize");

        assert_eq!(config.hook_adapter.timeout_ms, 45_000);
    }
}
