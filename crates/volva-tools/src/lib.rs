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
