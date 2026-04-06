use volva_config::VolvaConfig;

use crate::BackendRunRequest;

const ENVELOPE_HEADER: &str = "[volva-host-context]";
const USER_PROMPT_HEADER: &str = "[user-prompt]";
const HOST_NOTE: &str = "source: host-provided context from volva";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPrompt {
    final_prompt: String,
}

impl PreparedPrompt {
    #[must_use]
    pub fn final_prompt(&self) -> &str {
        &self.final_prompt
    }
}

#[must_use]
pub fn assemble_prompt(config: &VolvaConfig, request: &BackendRunRequest) -> PreparedPrompt {
    let mut lines = vec![
        ENVELOPE_HEADER.to_string(),
        HOST_NOTE.to_string(),
        format!("cwd: {}", request.cwd.display()),
        format!("backend: {}", request.backend),
    ];

    if !config.model.trim().is_empty() {
        lines.push(format!("model: {}", config.model.trim()));
    }

    let envelope = lines.join("\n");
    let final_prompt = format!("{envelope}\n\n{USER_PROMPT_HEADER}\n{}", request.prompt);

    PreparedPrompt { final_prompt }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use volva_config::VolvaConfig;
    use volva_core::BackendKind;

    use crate::BackendRunRequest;

    use super::assemble_prompt;

    #[test]
    fn assemble_prompt_prepends_static_host_envelope() {
        let config = VolvaConfig::default();
        let request = BackendRunRequest {
            prompt: "summarize the repository".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            backend: BackendKind::OfficialCli,
        };

        let prepared = assemble_prompt(&config, &request);

        assert_eq!(
            prepared.final_prompt(),
            "[volva-host-context]\n\
source: host-provided context from volva\n\
cwd: /tmp/project\n\
backend: official-cli\n\
model: claude-sonnet-4-6\n\n\
[user-prompt]\n\
summarize the repository"
        );
    }

    #[test]
    fn assemble_prompt_omits_blank_model_lines() {
        let mut config = VolvaConfig::default();
        config.model = "   ".to_string();
        let request = BackendRunRequest {
            prompt: "hello".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            backend: BackendKind::OfficialCli,
        };

        let prepared = assemble_prompt(&config, &request);

        assert!(prepared.final_prompt().contains("[user-prompt]\nhello"));
        assert!(!prepared.final_prompt().contains("\nmodel:"));
    }
}
