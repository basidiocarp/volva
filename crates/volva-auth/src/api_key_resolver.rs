use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use thiserror::Error;
use volva_core::{AuthMode, ResolvedCredential};

use crate::status::ENV_API_KEY;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("No API key found. Set ANTHROPIC_API_KEY or run: volva auth setup")]
    NotFound,
    #[error("Keychain error: {0}")]
    Keychain(String),
}

pub struct ApiKeyResolver;

impl ApiKeyResolver {
    /// Resolve API key through the following order:
    /// 1. `ANTHROPIC_API_KEY` environment variable
    /// 2. ~/.config/volva/config.toml `api_key` field
    /// 3. OS keychain
    pub fn resolve() -> Result<ResolvedCredential, AuthError> {
        // Step 1: Check environment variable
        if let Ok(api_key) = std::env::var(ENV_API_KEY)
            && !api_key.is_empty() {
                return Ok(ResolvedCredential {
                    mode: AuthMode::ApiKey,
                    secret: api_key,
                    source: ENV_API_KEY.to_string(),
                });
            }

        // Step 2: Check config file
        if let Ok(api_key) = Self::load_from_config() {
            return Ok(ResolvedCredential {
                mode: AuthMode::ApiKey,
                secret: api_key,
                source: "config-file".to_string(),
            });
        }

        // Step 3: Check OS keychain
        match Self::load_from_keychain() {
            Ok(api_key) => {
                return Ok(ResolvedCredential {
                    mode: AuthMode::ApiKey,
                    secret: api_key,
                    source: "os-keychain".to_string(),
                });
            }
            Err(AuthError::NotFound) => {} // Continue to error
            Err(e) => return Err(e),        // Surface keychain errors
        }

        Err(AuthError::NotFound)
    }

    /// Store API key in the OS keychain
    pub fn store_in_keychain(api_key: &str) -> Result<(), AuthError> {
        keyring::Entry::new("volva", "anthropic_api_key")
            .map_err(|e| AuthError::Keychain(e.to_string()))?
            .set_password(api_key)
            .map_err(|e| AuthError::Keychain(e.to_string()))?;

        Ok(())
    }

    fn load_from_config() -> Result<String> {
        let config_path = Self::config_file_path()?;
        if !config_path.exists() {
            anyhow::bail!("config file not found");
        }

        let content = fs::read_to_string(&config_path)
            .context("failed to read config file")?;

        // Simple line-by-line parser for api_key = "..." pattern
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("api_key") {
                let rest = rest.trim();
                if let Some(rest) = rest.strip_prefix('=') {
                    let rest = rest.trim();
                    if let Some(quoted) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                        if !quoted.is_empty() {
                            return Ok(quoted.to_string());
                        }
                    } else if let Some(quoted) = rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
                        && !quoted.is_empty() {
                            return Ok(quoted.to_string());
                        }
                }
            }
        }

        anyhow::bail!("api_key not found in config file");
    }

    fn load_from_keychain() -> Result<String, AuthError> {
        keyring::Entry::new("volva", "anthropic_api_key")
            .map_err(|e| AuthError::Keychain(e.to_string()))?
            .get_password()
            .map_err(|e| {
                if matches!(e, keyring::Error::NoEntry) {
                    AuthError::NotFound
                } else {
                    AuthError::Keychain(e.to_string())
                }
            })
    }

    fn config_file_path() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .context("could not determine home directory")?;
        Ok(home.join(".config").join("volva").join("config.toml"))
    }
}

