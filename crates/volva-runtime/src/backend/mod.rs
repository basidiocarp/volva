mod official_cli;

use anyhow::{Result, bail};
use volva_config::VolvaConfig;
use volva_core::BackendKind;

use crate::{BackendRunRequest, context::PreparedPrompt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

impl BackendRunResult {
    #[must_use]
    pub fn success(&self) -> bool {
        matches!(self.exit_code, Some(0))
    }
}

pub fn validate_request(request: &BackendRunRequest) -> Result<()> {
    match request.backend {
        BackendKind::OfficialCli => Ok(()),
        BackendKind::AnthropicApi => bail!(
            "backend `{}` is not available through `volva run` yet; use `volva chat` for the native API path",
            request.backend
        ),
        _ => bail!(
            "backend `{}` is not available through `volva run` yet; use `volva chat` for the native API path",
            request.backend
        ),
    }
}

pub fn run(
    config: &VolvaConfig,
    request: &BackendRunRequest,
    prepared_prompt: &PreparedPrompt,
) -> Result<BackendRunResult> {
    validate_request(request)?;

    match request.backend {
        BackendKind::OfficialCli => {
            official_cli::run(&config.backend.command, request, prepared_prompt)
        }
        BackendKind::AnthropicApi => unreachable!("validated unsupported run backend"),
        _ => unreachable!("validated unsupported run backend"),
    }
}
