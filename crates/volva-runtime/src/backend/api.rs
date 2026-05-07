use anyhow::{Context, Result};
use spore::logging::{SpanContext, tool_span};
use tokio::runtime::Runtime;
use volva_api::{ApiClientConfig, ChatRequest, chat_once};
use volva_auth::ApiKeyResolver;
use volva_config::VolvaConfig;

use crate::{BackendRunRequest, context::PreparedPrompt};

use super::BackendRunResult;

pub fn run(
    config: &VolvaConfig,
    request: &BackendRunRequest,
    prepared_prompt: &PreparedPrompt,
) -> Result<BackendRunResult> {
    let span_context = SpanContext::for_app("volva")
        .with_tool("api_backend")
        .with_session_id(request.session.session_id.as_str().to_string())
        .with_workspace_root(request.session.workspace.workspace_root.clone());
    let _tool_span = tool_span("api_backend", &span_context).entered();

    // Resolve API key through env/config/keychain
    let credential = ApiKeyResolver::resolve()
        .context("failed to resolve API key for native Anthropic API backend")?;

    // Build API client config from VolvaConfig to respect user overrides
    let client_config = ApiClientConfig {
        base_url: config.api_base_url.clone(),
        model: config.model.clone(),
    };

    // Create a Tokio runtime to run the async API call
    let runtime = Runtime::new().context("failed to create Tokio runtime for API backend")?;

    // Build the chat request
    let chat_request = ChatRequest::new(
        prepared_prompt.final_prompt().to_string(),
        4096,
        request.session.clone(),
    );

    // Execute the API call
    let response = runtime.block_on(chat_once(&client_config, &credential, &chat_request))?;

    Ok(BackendRunResult {
        stdout: response.text,
        stderr: String::new(),
        exit_code: Some(0),
    })
}
