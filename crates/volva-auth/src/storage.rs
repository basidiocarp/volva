use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use volva_core::AuthProvider;

use crate::types::StoredAnthropicTokens;

#[must_use]
pub fn config_dir() -> PathBuf {
    // config_dir is used for informational display; fall back to a placeholder
    // path when home_dir is unavailable rather than silently using CWD.
    home_dir_or_placeholder().join(".volva")
}

#[must_use]
pub fn auth_dir() -> PathBuf {
    config_dir().join("auth")
}

#[must_use]
pub fn provider_tokens_path(provider: AuthProvider) -> PathBuf {
    // This infallible variant is kept for callers that only need the expected
    // path for display purposes.  I/O operations use provider_tokens_path_required.
    provider_tokens_path_from_base(&home_dir_or_placeholder(), provider)
}

#[must_use]
pub(crate) fn provider_tokens_path_from_base(base_dir: &Path, provider: AuthProvider) -> PathBuf {
    base_dir
        .join(".volva")
        .join("auth")
        .join(provider_filename(provider))
}

fn provider_tokens_path_required(provider: AuthProvider) -> Result<PathBuf> {
    let home = require_home_dir()?;
    Ok(provider_tokens_path_from_base(&home, provider))
}

pub fn load_tokens(provider: AuthProvider) -> Result<Option<StoredAnthropicTokens>> {
    let path = provider_tokens_path_required(provider)?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let tokens = serde_json::from_str::<StoredAnthropicTokens>(&raw)?;
    Ok(Some(tokens))
}

pub fn save_tokens(provider: AuthProvider, tokens: &StoredAnthropicTokens) -> Result<PathBuf> {
    let path = provider_tokens_path_required(provider)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let payload = serde_json::to_vec_pretty(tokens)?;
    write_secure_json(&path, &payload)?;
    Ok(path)
}

pub fn clear_tokens(provider: AuthProvider) -> Result<()> {
    let path = provider_tokens_path_required(provider)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Returns the user's home directory, or an error if it cannot be determined.
/// Used by I/O operations that must not fall back to CWD.
fn require_home_dir() -> Result<PathBuf> {
    dirs::home_dir().context(
        "could not determine the home directory; \
         auth token operations require a valid home directory",
    )
}

/// Returns the user's home directory, or a placeholder path that cannot
/// accidentally match a real CWD.  Used only for display/path-construction
/// callers that do not perform I/O.
fn home_dir_or_placeholder() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| {
        // Use an unmistakable placeholder so the path is never silently written
        // to the current working directory.
        PathBuf::from("<home-unavailable>")
    })
}

fn provider_filename(provider: AuthProvider) -> &'static str {
    match provider {
        AuthProvider::Anthropic => "anthropic.json",
        _ => "unknown-provider.json",
    }
}

fn write_secure_json(path: &Path, payload: &[u8]) -> Result<()> {
    let temp_path = temporary_path(path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp_path)?;
        file.write_all(payload)?;
        file.write_all(b"\n")?;
    }

    #[cfg(not(unix))]
    {
        let mut payload_with_newline = payload.to_vec();
        payload_with_newline.push(b'\n');
        fs::write(&temp_path, payload_with_newline)?;
    }

    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to atomically replace auth state at {}",
            path.display()
        )
    })?;

    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("auth.json");
    path.with_file_name(format!("{file_name}.tmp-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn anthropic_tokens_path_is_provider_namespaced() {
        let path =
            provider_tokens_path_from_base(Path::new("/tmp/volva-home"), AuthProvider::Anthropic);
        assert_eq!(
            path,
            Path::new("/tmp/volva-home/.volva/auth/anthropic.json")
        );
    }

    #[test]
    fn atomic_write_replaces_target_without_leaving_temporary_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("volva-auth-storage-{unique}"));
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");
        let path = temp_dir.join("anthropic.json");

        write_secure_json(&path, br#"{"ok":true}"#).expect("write should succeed");

        let raw = fs::read_to_string(&path).expect("written file should be readable");
        assert!(raw.contains(r#""ok":true"#));

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
