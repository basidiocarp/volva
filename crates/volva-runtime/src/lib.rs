mod backend;
pub mod checkpoint_sqlite;
pub mod context;
pub mod execenv;
pub mod hash_edit;
mod hooks;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use spore::logging::{SpanContext, workflow_span};
use volva_bridge::{BridgeConfig, bridge_status};
use volva_config::VolvaConfig;
use volva_core::{
    BackendKind, ExecutionSessionIdentity, ExecutionSessionState, RuntimeStatus, StatusLine,
};

pub use hooks::{
    HookAdapter, HookAdapterState, HookContext, HookEvent, HookPhase, HookShell,
    render_command_line,
};

pub use backend::{BackendRunResult, BackendSessionSurface, session_status_lines};

#[derive(Debug, Clone)]
pub struct RuntimeBootstrap {
    pub config: VolvaConfig,
    pub bridge: BridgeConfig,
    hooks: HookShell,
}

#[derive(Debug, Clone)]
pub struct BackendRunRequest {
    pub prompt: String,
    pub session: ExecutionSessionIdentity,
    pub capabilities: context::Capabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendStatus {
    pub kind: BackendKind,
    pub command: String,
}

impl RuntimeBootstrap {
    #[must_use]
    pub fn new(config: VolvaConfig) -> Self {
        let hooks = HookShell::configured(config.hook_adapter.clone());
        Self::with_hook_shell(config, hooks)
    }

    #[must_use]
    pub(crate) fn with_hook_shell(config: VolvaConfig, hooks: HookShell) -> Self {
        let bridge = BridgeConfig {
            enabled: config.experimental_bridge,
            ..BridgeConfig::default()
        };
        Self {
            config,
            bridge,
            hooks,
        }
    }

    #[must_use]
    pub fn with_hook_adapter<T>(config: VolvaConfig, adapter: T) -> Self
    where
        T: HookAdapter + 'static,
    {
        Self::with_hook_shell(config, HookShell::with_adapter(adapter))
    }

    #[must_use]
    pub fn status(&self) -> RuntimeStatus {
        RuntimeStatus {
            auth_ready: volva_auth::resolve_credential().is_some(),
            builtin_tool_count: volva_tools::builtin_specs().len(),
            adapter_count: volva_adapters::adapter_names().len(),
            bridge_enabled: self.bridge.enabled,
        }
    }

    #[must_use]
    pub fn status_lines(&self) -> Vec<StatusLine> {
        let status = self.status();
        let hook_adapter = self.hooks.adapter_state().status_value();
        vec![
            StatusLine::new("backend", self.config.backend.kind.to_string()),
            StatusLine::new("backend_command", self.config.backend.command.clone()),
            StatusLine::new("model", self.config.model.clone()),
            StatusLine::new("api_base_url", self.config.api_base_url.clone()),
            StatusLine::new("hook_adapter", hook_adapter),
            StatusLine::new("auth_ready", status.auth_ready.to_string()),
            StatusLine::new("builtin_tools", status.builtin_tool_count.to_string()),
            StatusLine::new("adapters", status.adapter_count.to_string()),
            StatusLine::new("bridge", bridge_status(&self.bridge)),
        ]
    }

    #[must_use]
    pub fn backend_status(&self) -> BackendStatus {
        BackendStatus {
            kind: self.config.backend.kind,
            command: self.config.backend.command.clone(),
        }
    }

    fn execution_session_path(&self) -> PathBuf {
        self.config
            .vendor_dir
            .join("volva")
            .join("execution-session.json")
    }

    pub fn persist_execution_session(
        &self,
        session: ExecutionSessionIdentity,
    ) -> Result<BackendSessionSurface> {
        let surface = backend::session_surface_for(&self.config, session);
        let path = self.execution_session_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create execution session directory `{}`",
                    parent.display()
                )
            })?;
        }
        let payload =
            serde_json::to_vec_pretty(&surface).context("failed to serialize execution session")?;
        fs::write(&path, payload).with_context(|| {
            format!(
                "failed to persist execution session snapshot at `{}`",
                path.display()
            )
        })?;
        Ok(surface)
    }

    pub fn load_execution_session(&self) -> Result<Option<BackendSessionSurface>> {
        let path = self.execution_session_path();
        if !path.exists() {
            return Ok(None);
        }

        let payload = fs::read(&path).with_context(|| {
            format!(
                "failed to read persisted execution session snapshot from `{}`",
                path.display()
            )
        })?;
        let mut surface = serde_json::from_slice::<BackendSessionSurface>(&payload)
            .context("failed to deserialize persisted execution session")?;

        // Backfill workspace_id from workspace_root if empty (backwards compatibility)
        if surface.session.workspace.workspace_id.is_empty() {
            surface.session.workspace.workspace_id =
                std::fs::canonicalize(&surface.session.workspace.workspace_root)
                    .ok()
                    .and_then(|p| p.to_str().map(ToString::to_string))
                    .unwrap_or_else(|| surface.session.workspace.workspace_root.clone());
        }

        Ok(Some(surface))
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn hook_events(&self) -> Vec<HookEvent> {
        self.hooks.events()
    }

    fn flush_hook_diagnostics(&self) {
        for diagnostic in self.hooks.take_diagnostics() {
            eprintln!("warning: {diagnostic}");
        }
    }

    pub fn run_backend(&self, request: &BackendRunRequest) -> Result<BackendRunResult> {
        let _workflow_span =
            workflow_span("run_backend", &span_context_for_request(request)).entered();
        backend::validate_request(request)?;

        // Check for existing active session in the same workspace
        if !self.config.allow_concurrent_workspace_sessions
            && let Some(existing_session) = self.load_execution_session()?
        {
            let existing_state = existing_session.session.state;
            let existing_workspace_id = &existing_session.session.workspace.workspace_id;
            let incoming_workspace_id = &request.session.workspace.workspace_id;

            // Consider Active, Paused, or Resumed states as "active"
            let is_active = matches!(
                existing_state,
                ExecutionSessionState::Active
                    | ExecutionSessionState::Paused
                    | ExecutionSessionState::Resumed
            );

            if existing_workspace_id == incoming_workspace_id && is_active {
                return Err(anyhow::anyhow!(
                    "workspace {} already has an active session ({}); end it before starting a new one",
                    existing_workspace_id,
                    existing_session.session.session_id
                ));
            }
        }

        self.persist_execution_session(request.session.clone())?;
        let prepared_prompt =
            context::assemble_prompt(&self.config, request, &request.capabilities);

        let context = HookContext::from_request(request, prepared_prompt.final_prompt());

        self.hooks.emit(HookPhase::SessionStart, context.clone());
        self.flush_hook_diagnostics();
        self.hooks
            .emit(HookPhase::BeforePromptSend, context.clone());
        self.flush_hook_diagnostics();

        let result = match backend::run(&self.config, request, &prepared_prompt) {
            Ok(result) => result,
            Err(error) => {
                let failure_context = context.with_error(error.to_string());
                self.persist_execution_session(failure_context.execution_session.clone())?;
                self.hooks
                    .emit(HookPhase::BackendFailed, failure_context.clone());
                self.flush_hook_diagnostics();
                self.hooks.emit(HookPhase::SessionEnd, failure_context);
                self.flush_hook_diagnostics();
                return Err(error);
            }
        };

        let completed_context = context.with_result(&result);
        self.persist_execution_session(completed_context.execution_session.clone())?;
        if result.success() {
            self.hooks
                .emit(HookPhase::ResponseComplete, completed_context.clone());
            self.flush_hook_diagnostics();
        } else {
            self.hooks
                .emit(HookPhase::BackendFailed, completed_context.clone());
            self.flush_hook_diagnostics();
        }
        self.hooks.emit(HookPhase::SessionEnd, completed_context);
        self.flush_hook_diagnostics();

        Ok(result)
    }
}

fn span_context_for_request(request: &BackendRunRequest) -> SpanContext {
    SpanContext::for_app("volva")
        .with_tool("run_backend")
        .with_session_id(request.session.session_id.as_str().to_string())
        .with_workspace_root(request.session.workspace.workspace_root.clone())
}

#[cfg(test)]
mod tests {
    use super::RuntimeBootstrap;
    #[cfg(not(windows))]
    use std::fs;
    #[cfg(not(windows))]
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    #[cfg(not(windows))]
    use std::time::{SystemTime, UNIX_EPOCH};
    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionIdentity,
        ExecutionSessionState, OperationMode, WorkspaceBinding,
    };

    use crate::{BackendRunRequest, HookAdapter, HookEvent, HookPhase, HookShell, context};

    fn test_session(cwd: &str, backend: BackendKind) -> ExecutionSessionIdentity {
        ExecutionSessionIdentity::new(
            ExecutionMode::Run,
            backend,
            WorkspaceBinding::from_root(cwd),
            ExecutionParticipantIdentity {
                participant_id: "operator@volva".to_string(),
                host_kind: "volva".to_string(),
            },
            ExecutionSessionState::Active,
        )
    }

    fn test_request(prompt: &str, cwd: &str, backend: BackendKind) -> BackendRunRequest {
        BackendRunRequest {
            prompt: prompt.to_string(),
            session: test_session(cwd, backend),
            capabilities: context::Capabilities {
                mode: OperationMode::Baseline,
                canopy_available: false,
            },
        }
    }

    #[cfg(not(windows))]
    fn unique_vendor_dir(label: &str) -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_millis();
        std::env::temp_dir().join(format!("volva-{label}-{millis}"))
    }

    #[derive(Debug, Default)]
    struct ForwardingHookAdapter {
        events: Arc<Mutex<Vec<HookEvent>>>,
    }

    impl HookAdapter for ForwardingHookAdapter {
        fn handle(&self, event: HookEvent) {
            self.events
                .lock()
                .expect("hook adapter mutex should not be poisoned")
                .push(event);
        }
    }

    #[test]
    fn status_lines_include_backend_information() {
        let runtime = RuntimeBootstrap::new(VolvaConfig::default());
        let lines = runtime.status_lines();

        assert!(lines.iter().any(|line| line.label == "backend"));
        assert!(lines.iter().any(|line| line.label == "backend_command"));
    }

    #[test]
    fn configured_hook_adapter_is_reported_as_configured_when_command_is_present() {
        let mut config = VolvaConfig::default();
        config.hook_adapter.enabled = true;
        config.hook_adapter.command = Some("cortina".to_string());
        config.hook_adapter.args = vec![
            "adapter".to_string(),
            "volva".to_string(),
            "hook-event".to_string(),
        ];

        let runtime = RuntimeBootstrap::new(config);
        let lines = runtime.status_lines();

        assert!(lines.iter().any(|line| {
            line.label == "hook_adapter"
                && line.value == "configured-external:cortina adapter volva hook-event"
        }));
    }

    #[test]
    fn injected_adapter_is_reported_as_active() {
        let runtime = RuntimeBootstrap::with_hook_adapter(
            VolvaConfig::default(),
            ForwardingHookAdapter::default(),
        );
        let lines = runtime.status_lines();

        assert!(
            lines
                .iter()
                .any(|line| line.label == "hook_adapter" && line.value == "active-injected")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_emits_success_hooks_in_order() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.allow_concurrent_workspace_sessions = true;

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let result = runtime
            .run_backend(&test_request(
                "headless ok",
                "/tmp",
                BackendKind::OfficialCli,
            ))
            .expect("echo backend should run");

        assert!(result.success());

        let phases = runtime
            .hook_events()
            .into_iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();

        assert_eq!(
            phases,
            vec![
                HookPhase::SessionStart,
                HookPhase::BeforePromptSend,
                HookPhase::ResponseComplete,
                HookPhase::SessionEnd,
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_passes_assembled_prompt_to_backend_command() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.allow_concurrent_workspace_sessions = true;

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let result = runtime
            .run_backend(&test_request(
                "show status",
                "/tmp",
                BackendKind::OfficialCli,
            ))
            .expect("echo backend should run");

        assert!(result.stdout.starts_with("-p [volva-host-context]"));
        assert!(result.stdout.contains("\nbackend: official-cli"));
        assert!(result.stdout.contains("\n[user-prompt]\nshow status"));
        assert_ne!(result.stdout, "-p show status");
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_emits_assembled_prompt_in_hook_context() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.allow_concurrent_workspace_sessions = true;

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        runtime
            .run_backend(&test_request(
                "show status",
                "/tmp",
                BackendKind::OfficialCli,
            ))
            .expect("echo backend should run");

        let before_prompt = runtime
            .hook_events()
            .into_iter()
            .find(|event| event.phase == HookPhase::BeforePromptSend)
            .expect("before prompt hook should be emitted");

        assert!(
            before_prompt
                .context
                .prompt_text
                .starts_with("[volva-host-context]")
        );
        assert!(
            before_prompt
                .context
                .prompt_text
                .contains("\n[user-prompt]\nshow status")
        );
        assert_ne!(before_prompt.context.prompt_text, "show status");
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_forwards_hooks_to_adapter_in_order() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.allow_concurrent_workspace_sessions = true;

        let adapter = ForwardingHookAdapter::default();
        let events = adapter.events.clone();
        let runtime = RuntimeBootstrap::with_hook_adapter(config, adapter);
        let result = runtime
            .run_backend(&test_request(
                "headless adapter",
                "/tmp",
                BackendKind::OfficialCli,
            ))
            .expect("echo backend should run");

        assert!(result.success());

        let phases = events
            .lock()
            .expect("hook adapter mutex should not be poisoned")
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();

        assert_eq!(
            phases,
            vec![
                HookPhase::SessionStart,
                HookPhase::BeforePromptSend,
                HookPhase::ResponseComplete,
                HookPhase::SessionEnd,
            ]
        );
    }

    #[test]
    fn run_backend_emits_failure_hooks_in_order() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/definitely/not/a/real/claude".to_string();
        config.allow_concurrent_workspace_sessions = true;

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let error = runtime
            .run_backend(&test_request(
                "headless fail",
                "/tmp",
                BackendKind::OfficialCli,
            ))
            .expect_err("missing backend command should fail");

        let events = runtime.hook_events();
        let phases = events.iter().map(|event| event.phase).collect::<Vec<_>>();

        assert_eq!(
            phases,
            vec![
                HookPhase::SessionStart,
                HookPhase::BeforePromptSend,
                HookPhase::BackendFailed,
                HookPhase::SessionEnd,
            ]
        );

        assert_eq!(events[2].context.error, Some(error.to_string()));
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_emits_failure_hooks_for_nonzero_exit() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/usr/bin/false".to_string();
        config.allow_concurrent_workspace_sessions = true;

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let result = runtime
            .run_backend(&test_request(
                "headless fail",
                "/tmp",
                BackendKind::OfficialCli,
            ))
            .expect("false backend should launch");

        assert!(!result.success());

        let phases = runtime
            .hook_events()
            .into_iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();

        assert_eq!(
            phases,
            vec![
                HookPhase::SessionStart,
                HookPhase::BeforePromptSend,
                HookPhase::BackendFailed,
                HookPhase::SessionEnd,
            ]
        );
    }

    #[test]
    fn native_api_backend_is_now_supported() {
        let mut config = VolvaConfig::default();
        config.allow_concurrent_workspace_sessions = true;
        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());

        // AnthropicApi backend should be accepted by validate_request, even though
        // it may fail later due to missing API key. The important thing is that it
        // passes validation and hook dispatch is initiated.
        let result = runtime.run_backend(&test_request(
            "test prompt",
            "/tmp",
            BackendKind::AnthropicApi,
        ));

        // Will fail due to missing API key, but the validation should pass.
        // We're checking that AnthropicApi is no longer rejected at the validation stage.
        match result {
            Err(e) => {
                assert!(
                    e.to_string().contains("failed to resolve API key")
                        || e.to_string().contains("No API key found"),
                    "AnthropicApi should fail at API resolution, not validation: {e}"
                );
            }
            Ok(_) => panic!("without a real API key, this should fail"),
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_rejects_second_session_in_same_workspace_by_default() {
        let vendor_dir = unique_vendor_dir("cardinality-guard");
        let workspace_dir = unique_vendor_dir("cardinality-workspace");
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.vendor_dir = vendor_dir.clone();
        config.allow_concurrent_workspace_sessions = false;

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let workspace_path = workspace_dir.to_string_lossy().to_string();

        // Create the workspace directory
        fs::create_dir_all(&workspace_dir).ok();

        let mut request = test_request("first prompt", &workspace_path, BackendKind::OfficialCli);

        // First session should succeed
        let first_result = runtime.run_backend(&request);
        assert!(first_result.is_ok(), "first session should succeed");

        // Manually set the persisted session to Active state to simulate an ongoing session
        // (in production, a long-running session would still be Active)
        let mut persisted = runtime
            .load_execution_session()
            .expect("session should load")
            .expect("session should exist");
        persisted.session.state = ExecutionSessionState::Active;
        runtime
            .persist_execution_session(persisted.session.clone())
            .expect("should re-persist with Active state");

        // Second session in same workspace should fail
        let second_request =
            test_request("second prompt", &workspace_path, BackendKind::OfficialCli);
        let second_result = runtime.run_backend(&second_request);
        assert!(
            second_result.is_err(),
            "second session in same workspace should fail"
        );
        assert!(
            second_result
                .unwrap_err()
                .to_string()
                .contains("already has an active session"),
            "error message should mention active session"
        );

        let _ = fs::remove_dir_all(vendor_dir);
        let _ = fs::remove_dir_all(workspace_dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_allows_second_session_when_config_permits() {
        let vendor_dir = unique_vendor_dir("cardinality-guard-override");
        let workspace_dir = unique_vendor_dir("cardinality-workspace-override");
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.vendor_dir = vendor_dir.clone();
        config.allow_concurrent_workspace_sessions = true;

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let workspace_path = workspace_dir.to_string_lossy().to_string();

        // Create the workspace directory
        fs::create_dir_all(&workspace_dir).ok();

        let request = test_request("first prompt", &workspace_path, BackendKind::OfficialCli);

        // First session should succeed
        let first_result = runtime.run_backend(&request);
        assert!(first_result.is_ok(), "first session should succeed");

        // Second session should succeed when config permits
        let second_request =
            test_request("second prompt", &workspace_path, BackendKind::OfficialCli);
        let second_result = runtime.run_backend(&second_request);
        assert!(
            second_result.is_ok(),
            "second session should succeed when allowed by config"
        );

        let _ = fs::remove_dir_all(vendor_dir);
        let _ = fs::remove_dir_all(workspace_dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn run_backend_persists_latest_execution_session_snapshot() {
        let vendor_dir = unique_vendor_dir("execution-session");
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.vendor_dir = vendor_dir.clone();

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let session = test_session("/tmp", BackendKind::OfficialCli);
        runtime
            .run_backend(&BackendRunRequest {
                prompt: "persist me".to_string(),
                session: session.clone(),
                capabilities: context::Capabilities {
                    mode: OperationMode::Baseline,
                    canopy_available: false,
                },
            })
            .expect("echo backend should run");

        let loaded = runtime
            .load_execution_session()
            .expect("execution session snapshot should load")
            .expect("execution session snapshot should exist");

        assert_eq!(loaded.session.session_id, session.session_id);
        assert_eq!(loaded.session.workspace.workspace_root, "/tmp");
        assert_eq!(loaded.session.state, ExecutionSessionState::Finished);

        let _ = fs::remove_dir_all(vendor_dir);
    }
}
