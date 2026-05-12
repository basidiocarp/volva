use volva_core::ToolSpec;

#[must_use]
pub fn builtin_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "Bash".to_string(),
            description: "Execute shell commands with runtime policy checks.".to_string(),
        },
        ToolSpec {
            name: "Read".to_string(),
            description: "Read file contents before deciding whether to call the API.".to_string(),
        },
        ToolSpec {
            name: "Write".to_string(),
            description: "Write full file content through the local runtime.".to_string(),
        },
        ToolSpec {
            name: "Edit".to_string(),
            description: "Apply targeted edits through the local runtime.".to_string(),
        },
        ToolSpec {
            name: "Glob".to_string(),
            description: "Find files that match a glob pattern.".to_string(),
        },
        ToolSpec {
            name: "Grep".to_string(),
            description: "Search file contents with a regular expression.".to_string(),
        },
        ToolSpec {
            name: "WebFetch".to_string(),
            description: "Fetch a remote page when explicitly requested.".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::builtin_specs;

    #[test]
    fn returns_seven_builtin_tools() {
        assert_eq!(builtin_specs().len(), 7);
    }

    #[test]
    fn bash_is_first() {
        assert_eq!(builtin_specs()[0].name, "Bash");
    }

    #[test]
    fn all_tools_have_non_empty_name_and_description() {
        for spec in builtin_specs() {
            assert!(!spec.name.is_empty(), "empty name");
            assert!(!spec.description.is_empty(), "empty description for {}", spec.name);
        }
    }

    #[test]
    fn no_duplicate_names() {
        let mut names: Vec<String> = builtin_specs().into_iter().map(|s| s.name).collect();
        let original_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(original_len, names.len());
    }

    #[test]
    fn contains_expected_tools() {
        let names: Vec<String> = builtin_specs().into_iter().map(|s| s.name).collect();
        for expected in ["Bash", "Read", "Write", "Edit", "Glob", "Grep", "WebFetch"] {
            assert!(names.iter().any(|n| n == expected), "missing tool: {expected}");
        }
    }
}
