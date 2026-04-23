use std::env;

use anyhow::{Result, bail};
use clap::Args;
use tracing::info_span;

use volva_config::VolvaConfig;
use volva_core::{BackendKind, ExecutionMode, ExecutionSessionState, OperationMode};
use volva_runtime::{BackendRunRequest, RuntimeBootstrap, context};

use crate::backend::BackendArg;
use crate::session::session_for_workspace;

#[derive(Debug, Args, PartialEq, Eq)]
pub struct RunCommand {
    #[arg(long, value_enum)]
    pub backend: Option<BackendArg>,

    #[arg(required = true)]
    pub prompt: Vec<String>,
}

#[allow(clippy::needless_pass_by_value)]
pub fn handle_run(command: RunCommand, mode: OperationMode) -> Result<()> {
    let prompt = command.prompt.join(" ").trim().to_string();
    if prompt.is_empty() {
        bail!("volva run requires a prompt");
    }

    let cwd = env::current_dir()?;
    let mut config = VolvaConfig::load_from(&cwd)?;
    if let Some(backend_override) = command.backend {
        config.backend.kind = BackendKind::from(backend_override);
    }

    // Build capabilities based on mode
    let capabilities = match mode {
        OperationMode::Orchestration => {
            let canopy_ok = std::process::Command::new("canopy")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !canopy_ok {
                bail!(
                    "orchestration mode requires canopy — start canopy first or use --mode baseline"
                );
            }
            context::Capabilities {
                mode: OperationMode::Orchestration,
                canopy_available: true,
            }
        }
        OperationMode::Baseline => context::Capabilities {
            mode: OperationMode::Baseline,
            canopy_available: false,
        },
    };

    let runtime = RuntimeBootstrap::new(config.clone());
    let session = session_for_workspace(
        &cwd,
        ExecutionMode::Run,
        config.backend.kind,
        ExecutionSessionState::Active,
    );

    let span = info_span!("volva.execution",
        execution_mode = "run",
        backend = tracing::field::Empty,
        session_id = tracing::field::Empty,
        workspace_root = tracing::field::Empty,
    );
    span.record("backend", session.backend.to_string());
    span.record("session_id", session.session_id.to_string());
    span.record("workspace_root", session.workspace.workspace_root.clone());
    let _enter = span.enter();

    let result = runtime.run_backend(&BackendRunRequest {
        prompt,
        session,
        capabilities,
    })?;

    if result.success() {
        if !result.stdout.is_empty() {
            println!("{}", result.stdout);
        }
        return Ok(());
    }

    if !result.stderr.is_empty() {
        eprintln!("{}", result.stderr);
    }
    if !result.stdout.is_empty() {
        eprintln!("{}", result.stdout);
    }

    match result.exit_code {
        Some(code) => bail!("official backend exited with status code {code}"),
        None => bail!("official backend exited without a status code"),
    }
}
