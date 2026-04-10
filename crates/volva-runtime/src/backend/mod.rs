mod official_cli;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use volva_config::VolvaConfig;
use volva_core::{BackendKind, ExecutionSessionIdentity, StatusLine};

use crate::{BackendRunRequest, context::PreparedPrompt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendSessionSurface {
    pub backend: BackendKind,
    pub backend_command: String,
    pub run_supported: bool,
    pub session: ExecutionSessionIdentity,
}

impl BackendRunResult {
    #[must_use]
    pub fn success(&self) -> bool {
        matches!(self.exit_code, Some(0))
    }
}

pub fn validate_request(request: &BackendRunRequest) -> Result<()> {
    match request.session.backend {
        BackendKind::OfficialCli => Ok(()),
        BackendKind::AnthropicApi => bail!(
            "backend `{}` is not available through `volva run` yet; use `volva chat` for the native API path",
            request.session.backend
        ),
        _ => bail!(
            "backend `{}` is not available through `volva run` yet; use `volva chat` for the native API path",
            request.session.backend
        ),
    }
}

pub fn run(
    config: &VolvaConfig,
    request: &BackendRunRequest,
    prepared_prompt: &PreparedPrompt,
) -> Result<BackendRunResult> {
    validate_request(request)?;

    match request.session.backend {
        BackendKind::OfficialCli => {
            official_cli::run(&config.backend.command, request, prepared_prompt)
        }
        BackendKind::AnthropicApi => unreachable!("validated unsupported run backend"),
        _ => unreachable!("validated unsupported run backend"),
    }
}

#[must_use]
pub fn session_surface_for(
    config: &VolvaConfig,
    session: ExecutionSessionIdentity,
) -> BackendSessionSurface {
    BackendSessionSurface {
        backend: session.backend,
        backend_command: config.backend.command.clone(),
        run_supported: matches!(config.backend.kind, BackendKind::OfficialCli),
        session,
    }
}

#[must_use]
pub fn session_status_lines(session: &ExecutionSessionIdentity) -> Vec<StatusLine> {
    vec![
        StatusLine::new("session_id", session.session_id.as_str()),
        StatusLine::new("mode", session.mode.to_string()),
        StatusLine::new("workspace_root", session.workspace.workspace_root.clone()),
        StatusLine::new(
            "worktree_id",
            session.workspace.worktree_id.as_deref().unwrap_or("none"),
        ),
        StatusLine::new("backend", session.backend.to_string()),
        StatusLine::new(
            "primary_participant",
            session.primary_participant.participant_id.clone(),
        ),
        StatusLine::new("primary_host_kind", session.primary_participant.host_kind.clone()),
        StatusLine::new("session_state", session.state.to_string()),
    ]
}
