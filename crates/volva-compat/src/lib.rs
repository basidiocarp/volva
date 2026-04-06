use std::path::PathBuf;

#[must_use]
pub fn claude_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

#[must_use]
pub fn import_candidates() -> Vec<PathBuf> {
    vec![
        claude_config_dir().join("settings.json"),
        claude_config_dir().join("CLAUDE.md"),
        claude_config_dir().join("oauth_tokens.json"),
    ]
}
