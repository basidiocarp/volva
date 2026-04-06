use std::env;

use anyhow::{Result, bail};
use clap::Args;

use volva_config::VolvaConfig;
use volva_core::BackendKind;
use volva_runtime::{BackendRunRequest, RuntimeBootstrap};

use crate::backend::BackendArg;

#[derive(Debug, Args, PartialEq, Eq)]
pub struct RunCommand {
    #[arg(long, value_enum)]
    pub backend: Option<BackendArg>,

    #[arg(required = true)]
    pub prompt: Vec<String>,
}

pub fn handle_run(command: RunCommand) -> Result<()> {
    let prompt = command.prompt.join(" ").trim().to_string();
    if prompt.is_empty() {
        bail!("volva run requires a prompt");
    }

    let cwd = env::current_dir()?;
    let mut config = VolvaConfig::load_from(&cwd)?;
    if let Some(backend_override) = command.backend {
        config.backend.kind = BackendKind::from(backend_override);
    }

    let runtime = RuntimeBootstrap::new(config.clone());
    let result = runtime.run_backend(&BackendRunRequest {
        prompt,
        cwd,
        backend: config.backend.kind,
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
