use std::process::Command;

use anyhow::{Context, Result};
use spore::logging::{SpanContext, subprocess_span, tool_span};

use crate::{BackendRunRequest, context::PreparedPrompt};

use super::BackendRunResult;

pub fn run(
    command: &str,
    request: &BackendRunRequest,
    prepared_prompt: &PreparedPrompt,
) -> Result<BackendRunResult> {
    let span_context = SpanContext::for_app("volva")
        .with_tool("official_cli_backend")
        .with_session_id(request.session.session_id.as_str().to_string())
        .with_workspace_root(request.session.workspace.workspace_root.clone());
    let _tool_span = tool_span("official_cli_backend", &span_context).entered();
    let args = build_args(prepared_prompt);
    let _subprocess_span = subprocess_span(command, &span_context).entered();
    let output = Command::new(command)
        .current_dir(&request.session.workspace.workspace_root)
        .args(&args)
        .output()
        .with_context(|| format!("failed to launch official Claude backend via `{command}`"))?;

    Ok(BackendRunResult {
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        exit_code: output.status.code(),
    })
}

fn build_args(prepared_prompt: &PreparedPrompt) -> Vec<String> {
    vec!["-p".to_string(), prepared_prompt.final_prompt().to_string()]
}

#[cfg(test)]
mod tests {
    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionIdentity,
        ExecutionSessionState, OperationMode, WorkspaceBinding,
    };

    use crate::{BackendRunRequest, context};

    use super::{build_args, run};

    fn test_session(workspace_root: &str) -> ExecutionSessionIdentity {
        ExecutionSessionIdentity::new(
            ExecutionMode::Run,
            BackendKind::OfficialCli,
            WorkspaceBinding::from_root(workspace_root),
            ExecutionParticipantIdentity {
                participant_id: "operator@volva".to_string(),
                host_kind: "volva".to_string(),
            },
            ExecutionSessionState::Active,
        )
    }

    fn test_request(prompt: &str, workspace_root: &str) -> BackendRunRequest {
        BackendRunRequest {
            prompt: prompt.to_string(),
            session: test_session(workspace_root),
            capabilities: context::Capabilities {
                mode: OperationMode::Baseline,
                canopy_available: false,
            },
        }
    }

    #[test]
    fn build_args_uses_print_mode_with_assembled_prompt_payload() {
        let request = test_request("summarize the repo", "/tmp");
        let prepared = crate::context::assemble_prompt(
            &VolvaConfig::default(),
            &request,
            &request.capabilities,
        );
        let args = build_args(&prepared);

        assert_eq!(
            args,
            vec!["-p".to_string(), prepared.final_prompt().to_string()]
        );
    }

    #[test]
    fn missing_command_returns_launch_error() {
        let request = test_request("hello", "/tmp");
        let prepared = crate::context::assemble_prompt(
            &VolvaConfig::default(),
            &request,
            &request.capabilities,
        );
        let error = run("/definitely/not/a/real/claude", &request, &prepared)
            .expect_err("missing backend command should fail");

        assert!(
            error
                .to_string()
                .contains("failed to launch official Claude backend"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn successful_command_captures_stdout_and_exit_code() {
        let request = test_request("headless ok", "/tmp");
        let prepared = crate::context::assemble_prompt(
            &VolvaConfig::default(),
            &request,
            &request.capabilities,
        );
        let result = run("/bin/echo", &request, &prepared).expect("echo command should run");

        assert!(result.stdout.starts_with("-p [volva-host-context]"));
        assert!(result.stdout.contains("\n[user-prompt]\nheadless ok"));
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stderr, "");
    }

    #[test]
    fn launched_command_can_exit_nonzero() {
        let request = test_request("headless fail", "/tmp");
        let prepared = crate::context::assemble_prompt(
            &VolvaConfig::default(),
            &request,
            &request.capabilities,
        );
        let result =
            run("/usr/bin/false", &request, &prepared).expect("false command should launch");

        assert_eq!(result.exit_code, Some(1));
        assert!(!result.success());
    }
}
