use std::env;

use anyhow::{Context, Result, bail};
use clap::Args;
use tokio::runtime::Runtime;
use tracing::info_span;
use volva_api::{ApiClientConfig, ChatRequest};
use volva_auth::resolve_credential;
use volva_config::VolvaConfig;
use volva_core::{BackendKind, ExecutionMode, ExecutionSessionState, OperationMode};
use volva_runtime::RuntimeBootstrap;

use crate::session::session_for_workspace;

const DEFAULT_CHAT_MAX_TOKENS: u32 = 1024;

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ChatCommand {
    #[arg(required = true)]
    pub prompt: Vec<String>,
}

#[allow(clippy::needless_pass_by_value)]
pub fn handle_chat(command: ChatCommand, mode: OperationMode) -> Result<()> {
    let prompt = command.prompt.join(" ").trim().to_string();
    if prompt.is_empty() {
        bail!("volva chat requires a prompt");
    }

    let credential = resolve_credential().context(
        "Anthropic auth is not available. Run `volva auth login anthropic` or set ANTHROPIC_API_KEY.",
    )?;

    let root = env::current_dir()?;
    let config = VolvaConfig::load_from(&root)?;
    let host_runtime = RuntimeBootstrap::new(config.clone());
    let session = session_for_workspace(
        &root,
        ExecutionMode::Chat,
        BackendKind::AnthropicApi,
        ExecutionSessionState::Active,
    );

    // Note: chat uses the API path directly, not run_backend, so capabilities
    // are created but not used. Future integration with Hymenium will use them.
    match mode {
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
        }
        OperationMode::Baseline => {}
    }

    let span = info_span!(
        "volva.execution",
        execution_mode = "chat",
        backend = tracing::field::Empty,
        session_id = tracing::field::Empty,
        workspace_root = tracing::field::Empty,
    );
    span.record("backend", session.backend.to_string());
    span.record("session_id", session.session_id.to_string());
    span.record("workspace_root", session.workspace.workspace_root.clone());
    let _enter = span.enter();
    let api_config = ApiClientConfig {
        base_url: config.api_base_url,
        model: config.model,
    };
    let request = ChatRequest {
        prompt,
        max_tokens: DEFAULT_CHAT_MAX_TOKENS,
        session: session.clone(),
    };

    persist_chat_session_state(&host_runtime, &session, ExecutionSessionState::Active)?;
    let runtime_persist = host_runtime.clone();
    let session_for_state = session.clone();
    let runtime = Runtime::new()?;
    let response = runtime.block_on(volva_api::chat_once_with_state_observer(
        &api_config,
        &credential,
        &request,
        move |state| {
            let _ = persist_chat_session_state(&runtime_persist, &session_for_state, state);
        },
    ));

    match response {
        Ok(response) => {
            persist_chat_session_state(&host_runtime, &session, ExecutionSessionState::Finished)?;
            println!("{}", response.text);
            Ok(())
        }
        Err(error) => {
            persist_chat_session_state(&host_runtime, &session, ExecutionSessionState::Finished)?;
            Err(error)
        }
    }
}

fn persist_chat_session_state(
    runtime: &RuntimeBootstrap,
    session: &volva_core::ExecutionSessionIdentity,
    state: ExecutionSessionState,
) -> Result<()> {
    runtime.persist_execution_session(session.clone().with_state(state))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionIdentity,
        ExecutionSessionState, WorkspaceBinding,
    };
    use volva_runtime::RuntimeBootstrap;

    use super::persist_chat_session_state;

    fn unique_vendor_dir(label: &str) -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_millis();
        std::env::temp_dir().join(format!("volva-chat-{label}-{millis}"))
    }

    fn test_chat_session(workspace_root: &str) -> ExecutionSessionIdentity {
        ExecutionSessionIdentity::new(
            ExecutionMode::Chat,
            BackendKind::AnthropicApi,
            WorkspaceBinding::from_root(workspace_root),
            ExecutionParticipantIdentity {
                participant_id: "operator@volva".to_string(),
                host_kind: "volva".to_string(),
            },
            ExecutionSessionState::Active,
        )
    }

    #[test]
    fn persisted_chat_retry_transitions_capture_paused_then_resumed() {
        let vendor_dir = unique_vendor_dir("retry-state");
        let config = VolvaConfig {
            vendor_dir: vendor_dir.clone(),
            ..Default::default()
        };
        let runtime = RuntimeBootstrap::new(config);
        let session = test_chat_session("/tmp/chat-retry");

        persist_chat_session_state(&runtime, &session, ExecutionSessionState::Active)
            .expect("active chat session should persist");
        persist_chat_session_state(&runtime, &session, ExecutionSessionState::Paused)
            .expect("paused chat session should persist");

        let paused = runtime
            .load_execution_session()
            .expect("paused snapshot should load")
            .expect("paused snapshot should exist");
        assert_eq!(paused.session.session_id, session.session_id);
        assert_eq!(paused.session.state, ExecutionSessionState::Paused);

        persist_chat_session_state(&runtime, &session, ExecutionSessionState::Resumed)
            .expect("resumed chat session should persist");

        let resumed = runtime
            .load_execution_session()
            .expect("resumed snapshot should load")
            .expect("resumed snapshot should exist");
        assert_eq!(resumed.session.session_id, session.session_id);
        assert_eq!(resumed.session.state, ExecutionSessionState::Resumed);
        assert_eq!(resumed.session.workspace.workspace_root, "/tmp/chat-retry");

        let _ = fs::remove_dir_all(vendor_dir);
    }
}
