use std::{
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use spore::logging::{SpanContext, subprocess_span, tool_span};

use crate::{BackendRunRequest, context::PreparedPrompt};

use super::BackendRunResult;

/// Hard deadline for the official CLI backend subprocess.
///
/// After this duration the child process is killed and an error is returned.
/// The value is intentionally conservative: interactive Claude CLI sessions
/// are expected to complete within a few minutes; anything that runs much
/// longer is more likely a hang than useful work.
const BACKEND_SUBPROCESS_TIMEOUT: Duration = Duration::from_mins(1);

/// Poll interval while waiting for the backend subprocess to exit.
const BACKEND_SUBPROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub fn run(
    command: &str,
    request: &BackendRunRequest,
    prepared_prompt: &PreparedPrompt,
) -> Result<BackendRunResult> {
    run_with_timeout(
        command,
        request,
        prepared_prompt,
        BACKEND_SUBPROCESS_TIMEOUT,
    )
}

fn run_with_timeout(
    command: &str,
    request: &BackendRunRequest,
    prepared_prompt: &PreparedPrompt,
    timeout: Duration,
) -> Result<BackendRunResult> {
    let span_context = SpanContext::for_app("volva")
        .with_tool("official_cli_backend")
        .with_session_id(request.session.session_id.as_str().to_string())
        .with_workspace_root(request.session.workspace.workspace_root.clone());
    let _tool_span = tool_span("official_cli_backend", &span_context).entered();
    let args = build_args(prepared_prompt);
    let _subprocess_span = subprocess_span(command, &span_context).entered();

    let mut child = Command::new(command)
        .current_dir(&request.session.workspace.workspace_root)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to launch official Claude backend via `{command}`"))?;

    // Collect stdout and stderr on background threads so that the pipes do not
    // block the child from making progress while we poll for exit.
    let mut stdout_pipe = child.stdout.take().expect("stdout was piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped");

    let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>();
    let (stderr_tx, stderr_rx) = mpsc::channel::<Vec<u8>>();

    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        let _ = stdout_tx.send(buf);
    });
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        let _ = stderr_tx.send(buf);
    });

    let start = Instant::now();
    let exit_status = loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to poll official Claude backend process state")?
        {
            break status;
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "official Claude backend `{command}` timed out after {timeout:?}; \
                 the process was killed"
            );
        }

        thread::sleep(BACKEND_SUBPROCESS_POLL_INTERVAL);
    };

    // Collect the buffered output; the reader threads finish once the child exits.
    let stdout_bytes = stdout_rx.recv().unwrap_or_default();
    let stderr_bytes = stderr_rx.recv().unwrap_or_default();

    Ok(BackendRunResult {
        stdout: String::from_utf8_lossy(&stdout_bytes).trim().to_string(),
        stderr: String::from_utf8_lossy(&stderr_bytes).trim().to_string(),
        exit_code: exit_status.code(),
    })
}

fn build_args(prepared_prompt: &PreparedPrompt) -> Vec<String> {
    vec!["-p".to_string(), prepared_prompt.final_prompt().to_string()]
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::time::Duration;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionIdentity,
        ExecutionSessionState, OperationMode, WorkspaceBinding,
    };

    use crate::{BackendRunRequest, context};

    #[cfg(unix)]
    use super::run_with_timeout;
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

    #[cfg(unix)]
    #[test]
    fn official_cli_timeout_kills_hung_process_and_returns_error() {
        // Use a 3-second timeout against a shell command that sleeps for much
        // longer. /bin/sh accepts arbitrary arguments (ignoring extras), so the
        // `-p <prompt>` args that build_args() appends are harmlessly ignored
        // by the shell's positional-parameter handling; the `sleep 120` from
        // the -c script is what actually runs.
        //
        // We use /bin/sh -c 'sleep 120' rather than /bin/sleep directly because
        // BSD sleep (macOS) rejects the `-p` flag that build_args() prepends.
        let request = test_request("timeout test", "/tmp");
        let prepared = crate::context::assemble_prompt(
            &VolvaConfig::default(),
            &request,
            &request.capabilities,
        );

        // Build a small wrapper script so the command ignores extra arguments.
        let script_path = {
            let path = std::env::temp_dir().join("volva-test-sleep-backend.sh");
            std::fs::write(&path, "#!/bin/sh\nsleep 120\n").expect("should write test script");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("should set script executable");
            path
        };

        let error = run_with_timeout(
            script_path
                .to_str()
                .expect("script path should be valid UTF-8"),
            &request,
            &prepared,
            Duration::from_secs(3),
        )
        .expect_err("hung backend should time out");

        let _ = std::fs::remove_file(&script_path);

        assert!(
            error.to_string().contains("timed out"),
            "unexpected error: {error}"
        );
    }
}
