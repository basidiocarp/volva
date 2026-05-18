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

#[cfg(test)]
mod tests {
    use super::{claude_config_dir, import_candidates};

    #[test]
    fn config_dir_ends_with_claude() {
        let dir = claude_config_dir();
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some(".claude"));
    }

    #[test]
    fn config_dir_has_a_parent() {
        assert!(claude_config_dir().parent().is_some());
    }

    #[test]
    fn import_candidates_returns_only_existing_files() {
        // This test may return 0-3 depending on whether files exist.
        // We only assert that it's a valid count and all returned paths exist.
        let candidates = import_candidates();
        assert!(candidates.len() <= 3);
        for path in candidates {
            assert!(path.exists(), "candidate {} should exist", path.display());
        }
    }

    #[test]
    fn import_candidates_all_under_config_dir() {
        let base = claude_config_dir();
        for path in import_candidates() {
            assert!(
                path.starts_with(&base),
                "{} is not under {}",
                path.display(),
                base.display()
            );
        }
    }

    #[test]
    fn all_returned_candidates_exist() {
        for candidate in import_candidates() {
            assert!(
                candidate.exists(),
                "all returned candidates must exist, but {} does not",
                candidate.display()
            );
        }
    }

    #[test]
    fn import_candidates_returns_recognized_filenames_when_they_exist() {
        let candidates = import_candidates();
        let names: Vec<&str> = candidates
            .iter()
            .filter_map(|p| p.file_name()?.to_str())
            .collect();
        // Each name that is returned must be one of the expected filenames.
        for name in names {
            assert!(
                matches!(name, "settings.json" | "CLAUDE.md" | "oauth_tokens.json"),
                "unexpected filename in candidates: {name}"
            );
        }
    }
}
