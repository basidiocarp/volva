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
    fn import_candidates_returns_three_files() {
        assert_eq!(import_candidates().len(), 3);
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
    fn import_candidates_expected_filenames() {
        let candidates = import_candidates();
        let names: Vec<&str> = candidates
            .iter()
            .filter_map(|p| p.file_name()?.to_str())
            .collect();
        assert!(names.contains(&"settings.json"));
        assert!(names.contains(&"CLAUDE.md"));
        assert!(names.contains(&"oauth_tokens.json"));
    }
}
