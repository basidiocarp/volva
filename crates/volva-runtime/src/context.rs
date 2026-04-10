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
        format!("session_id: {}", request.session.session_id),
        format!("workspace_root: {}", request.session.workspace.workspace_root),
        format!(
            "worktree_id: {}",
            request
                .session
                .workspace
                .worktree_id
                .as_deref()
                .unwrap_or("none")
        ),
        format!("backend: {}", request.session.backend),
        format!("mode: {}", request.session.mode),
        format!(
            "participant: {}",
            request.session.primary_participant.participant_id
        ),
        format!("session_state: {}", request.session.state),
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
    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionId,
        ExecutionSessionIdentity, ExecutionSessionState, WorkspaceBinding,
    };

    use crate::BackendRunRequest;

    use super::assemble_prompt;

    #[test]
    fn assemble_prompt_prepends_static_host_envelope() {
        let config = VolvaConfig::default();
        let request = BackendRunRequest {
            prompt: "summarize the repository".to_string(),
            session: ExecutionSessionIdentity {
                session_id: ExecutionSessionId("volva-run-test".to_string()),
                mode: ExecutionMode::Run,
                backend: BackendKind::OfficialCli,
                workspace: WorkspaceBinding::from_root("/tmp/project"),
                primary_participant: ExecutionParticipantIdentity {
                    participant_id: "operator@volva".to_string(),
                    host_kind: "volva".to_string(),
                },
                state: ExecutionSessionState::Active,
            },
        };

        let prepared = assemble_prompt(&config, &request);

        assert_eq!(
            prepared.final_prompt(),
            "[volva-host-context]\n\
source: host-provided context from volva\n\
session_id: volva-run-test\n\
workspace_root: /tmp/project\n\
worktree_id: none\n\
backend: official-cli\n\
mode: run\n\
participant: operator@volva\n\
session_state: active\n\
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
            session: ExecutionSessionIdentity {
                session_id: ExecutionSessionId("volva-run-test".to_string()),
                mode: ExecutionMode::Run,
                backend: BackendKind::OfficialCli,
                workspace: WorkspaceBinding::from_root("/tmp/project"),
                primary_participant: ExecutionParticipantIdentity {
                    participant_id: "operator@volva".to_string(),
                    host_kind: "volva".to_string(),
                },
                state: ExecutionSessionState::Active,
            },
        };

        let prepared = assemble_prompt(&config, &request);

        assert!(prepared.final_prompt().contains("[user-prompt]\nhello"));
        assert!(!prepared.final_prompt().contains("\nmodel:"));
    }
}
