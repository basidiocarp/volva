use std::env;

use anyhow::{Context, Result, bail};
use clap::Args;
use tokio::runtime::Runtime;
use volva_api::{ApiClientConfig, ChatRequest, chat_once};
use volva_auth::resolve_credential;
use volva_config::VolvaConfig;

const DEFAULT_CHAT_MAX_TOKENS: u32 = 1024;

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ChatCommand {
    #[arg(required = true)]
    pub prompt: Vec<String>,
}

pub fn handle_chat(command: ChatCommand) -> Result<()> {
    let prompt = command.prompt.join(" ").trim().to_string();
    if prompt.is_empty() {
        bail!("volva chat requires a prompt");
    }

    let credential = resolve_credential().context(
        "Anthropic auth is not available. Run `volva auth login anthropic` or set ANTHROPIC_API_KEY.",
    )?;

    let root = env::current_dir()?;
    let config = VolvaConfig::load_from(&root)?;
    let api_config = ApiClientConfig {
        base_url: config.api_base_url,
        model: config.model,
    };
    let request = ChatRequest {
        prompt,
        max_tokens: DEFAULT_CHAT_MAX_TOKENS,
    };

    let runtime = Runtime::new()?;
    let response = runtime.block_on(chat_once(&api_config, &credential, &request))?;

    println!("{}", response.text);
    Ok(())
}
