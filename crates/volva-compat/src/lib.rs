//! Backwards-compatibility layer for Claude configuration.
//!
//! This crate provides discovery of legacy Claude configuration files that exist on disk.
//! Only paths that actually exist are included in the candidate list.
//! Future versions may add migration flow and consolidated settings model.

use std::path::PathBuf;

#[must_use]
pub fn claude_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

#[must_use]
pub fn import_candidates() -> Vec<PathBuf> {
    let base = claude_config_dir();
    vec![
        base.join("settings.json"),
        base.join("CLAUDE.md"),
        base.join("oauth_tokens.json"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}
