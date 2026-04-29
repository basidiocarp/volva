use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use volva_core::{BackendKind, CheckpointDurability};

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
    /// Whether this adapter is explicitly trusted to receive hook events.
    ///
    /// Adapters whose command name contains "cortina" are implicitly trusted
    /// regardless of this field. All other project-local adapters must set
    /// `trusted: true` to suppress the runtime warning.
    #[serde(default)]
    pub trusted: bool,
}

impl HookAdapterConfig {
    /// Clamp `timeout_ms` to the valid range [1, 30000].
    #[must_use]
    pub fn with_clamped_timeout(mut self) -> Self {
        self.timeout_ms = self.timeout_ms.clamp(1, 30_000);
        self
    }

    /// Return `true` if this adapter is trusted.
    ///
    /// An adapter is trusted when `trusted: true` is set in config, OR when the
    /// adapter command name contains `"cortina"` (the recognized ecosystem adapter).
    #[must_use]
    pub fn is_trusted(&self, adapter_name: &str) -> bool {
        self.trusted || adapter_name.contains("cortina")
    }
}

impl Default for HookAdapterConfig {
    fn default() -> Self {
        Self {
            enabled: default_hook_adapter_enabled(),
            command: None,
            args: Vec::new(),
            timeout_ms: default_hook_adapter_timeout_ms(),
            trusted: false,
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
    #[serde(default)]
    pub durability_mode: CheckpointDurability,
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
            durability_mode: CheckpointDurability::default(),
        }
    }
}

impl VolvaConfig {
    pub fn load_from(root: &Path) -> Result<Self> {
        let path = root.join("volva.json");
        let mut config = if path.exists() {
            let raw = fs::read_to_string(path)?;
            serde_json::from_str::<Self>(&raw)?
        } else {
            Self::default()
        };

        if config.vendor_dir.is_relative() {
            config.vendor_dir = root.join(&config.vendor_dir);
        }

        // Check for VOLVA_CHECKPOINT_DURABILITY env var override
        if let Ok(mode_str) = std::env::var("VOLVA_CHECKPOINT_DURABILITY") {
            Self::apply_durability_override(&mut config, &mode_str);
        }

        Ok(config)
    }

    fn apply_durability_override(config: &mut VolvaConfig, mode_str: &str) {
        match mode_str {
            "sync" => config.durability_mode = CheckpointDurability::Sync,
            "async" => config.durability_mode = CheckpointDurability::Async,
            "exit" => config.durability_mode = CheckpointDurability::Exit,
            other => {
                eprintln!("warning: unknown checkpoint durability mode '{other}', keeping default");
            }
        }
    }
}

/// Global user-level volva configuration, stored at `~/.config/volva/config.toml`.
/// Written by `stipe init`. The `--mode` CLI flag takes precedence over this.
#[derive(Debug, Default)]
pub struct GlobalVolvaConfig {
    pub mode: Option<String>,
}

impl GlobalVolvaConfig {
    /// Load from `~/.config/volva/config.toml`. Returns defaults if the file is absent or unreadable.
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = dirs::config_dir().map(|d| d.join("volva").join("config.toml")) else {
            return Self::default();
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let mode = contents.lines().find_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("mode")?.trim_start();
            let value = rest.strip_prefix('=')?.trim().trim_matches('"');
            Some(value.to_string())
        });
        Self { mode }
    }

    /// Convert the stored mode string to an `OperationMode`, if recognized.
    #[must_use]
    pub fn operation_mode(&self) -> Option<volva_core::OperationMode> {
        match self.mode.as_deref() {
            Some("baseline") => Some(volva_core::OperationMode::Baseline),
            Some("orchestration") => Some(volva_core::OperationMode::Orchestration),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendConfig, GlobalVolvaConfig, HookAdapterConfig, VolvaConfig};
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

    #[test]
    fn global_config_defaults_when_missing() {
        let config = GlobalVolvaConfig::default();
        assert_eq!(config.mode, None);
        assert_eq!(config.operation_mode(), None);
    }

    #[test]
    fn global_config_parses_baseline_mode() {
        let config = GlobalVolvaConfig {
            mode: Some("baseline".to_string()),
        };
        assert_eq!(
            config.operation_mode(),
            Some(volva_core::OperationMode::Baseline)
        );
    }

    #[test]
    fn global_config_parses_orchestration_mode() {
        let config = GlobalVolvaConfig {
            mode: Some("orchestration".to_string()),
        };
        assert_eq!(
            config.operation_mode(),
            Some(volva_core::OperationMode::Orchestration)
        );
    }

    #[test]
    fn global_config_ignores_unknown_mode() {
        let config = GlobalVolvaConfig {
            mode: Some("unknown".to_string()),
        };
        assert_eq!(config.operation_mode(), None);
    }

    #[test]
    fn durability_mode_default_is_async() {
        let config = VolvaConfig::default();
        assert_eq!(
            config.durability_mode,
            volva_core::CheckpointDurability::Async
        );
    }

    #[test]
    fn durability_mode_deserializes_from_json() {
        let config = serde_json::from_str::<VolvaConfig>(
            r#"{
              "model": "claude-sonnet-4-6",
              "api_base_url": "https://api.anthropic.com",
              "experimental_bridge": false,
              "durability_mode": "sync"
            }"#,
        )
        .expect("config should deserialize");

        assert_eq!(
            config.durability_mode,
            volva_core::CheckpointDurability::Sync
        );
    }

    #[test]
    fn durability_override_sync() {
        let mut config = VolvaConfig::default();
        VolvaConfig::apply_durability_override(&mut config, "sync");
        assert_eq!(
            config.durability_mode,
            volva_core::CheckpointDurability::Sync
        );
    }

    #[test]
    fn durability_override_async() {
        let mut config = VolvaConfig {
            durability_mode: volva_core::CheckpointDurability::Sync,
            ..Default::default()
        };
        VolvaConfig::apply_durability_override(&mut config, "async");
        assert_eq!(
            config.durability_mode,
            volva_core::CheckpointDurability::Async
        );
    }

    #[test]
    fn durability_override_exit() {
        let mut config = VolvaConfig::default();
        VolvaConfig::apply_durability_override(&mut config, "exit");
        assert_eq!(
            config.durability_mode,
            volva_core::CheckpointDurability::Exit
        );
    }

    #[test]
    fn durability_override_unknown_keeps_current() {
        let mut config = VolvaConfig {
            durability_mode: volva_core::CheckpointDurability::Sync,
            ..Default::default()
        };
        VolvaConfig::apply_durability_override(&mut config, "invalid");
        assert_eq!(
            config.durability_mode,
            volva_core::CheckpointDurability::Sync
        );
    }

    #[test]
    fn hook_adapter_timeout_zero_is_clamped_to_one() {
        let config = HookAdapterConfig {
            enabled: true,
            command: Some("cortina".to_string()),
            args: Vec::new(),
            timeout_ms: 0,
            trusted: false,
        };

        let clamped = config.with_clamped_timeout();
        assert_eq!(clamped.timeout_ms, 1);
    }

    #[test]
    fn hook_adapter_timeout_exceeding_max_is_clamped() {
        let config = HookAdapterConfig {
            enabled: true,
            command: Some("cortina".to_string()),
            args: Vec::new(),
            timeout_ms: 50_000,
            trusted: false,
        };

        let clamped = config.with_clamped_timeout();
        assert_eq!(clamped.timeout_ms, 30_000);
    }

    #[test]
    fn hook_adapter_timeout_within_range_is_unchanged() {
        let config = HookAdapterConfig {
            enabled: true,
            command: Some("cortina".to_string()),
            args: Vec::new(),
            timeout_ms: 15_000,
            trusted: false,
        };

        let clamped = config.with_clamped_timeout();
        assert_eq!(clamped.timeout_ms, 15_000);
    }
}
