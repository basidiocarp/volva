mod backend;
mod context;
mod hooks;

use std::path::PathBuf;

use anyhow::Result;
use volva_bridge::{BridgeConfig, bridge_status};
use volva_config::VolvaConfig;
use volva_core::{BackendKind, RuntimeStatus, StatusLine};

pub use hooks::{
    HookAdapter, HookAdapterState, HookContext, HookEvent, HookPhase, HookShell,
    render_command_line,
};

pub use backend::BackendRunResult;

#[derive(Debug, Clone)]
pub struct RuntimeBootstrap {
    pub config: VolvaConfig,
    pub bridge: BridgeConfig,
    hooks: HookShell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRunRequest {
    pub prompt: String,
    pub cwd: PathBuf,
    pub backend: BackendKind,
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
        backend::validate_request(request)?;
        let prepared_prompt = context::assemble_prompt(&self.config, request);

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
                self.hooks
                    .emit(HookPhase::BackendFailed, failure_context.clone());
                self.flush_hook_diagnostics();
                self.hooks.emit(HookPhase::SessionEnd, failure_context);
                self.flush_hook_diagnostics();
                return Err(error);
            }
        };

        let completed_context = context.with_result(&result);
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

#[cfg(test)]
mod tests {
    use super::RuntimeBootstrap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use volva_config::VolvaConfig;
    use volva_core::BackendKind;

    use crate::{BackendRunRequest, HookAdapter, HookEvent, HookPhase, HookShell};

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

    #[test]
    fn run_backend_emits_success_hooks_in_order() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let result = runtime
            .run_backend(&BackendRunRequest {
                prompt: "headless ok".to_string(),
                cwd: PathBuf::from("/tmp"),
                backend: BackendKind::OfficialCli,
            })
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

    #[test]
    fn run_backend_passes_assembled_prompt_to_backend_command() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let result = runtime
            .run_backend(&BackendRunRequest {
                prompt: "show status".to_string(),
                cwd: PathBuf::from("/tmp"),
                backend: BackendKind::OfficialCli,
            })
            .expect("echo backend should run");

        assert!(result.stdout.starts_with("-p [volva-host-context]"));
        assert!(result.stdout.contains("\nbackend: official-cli"));
        assert!(result.stdout.contains("\n[user-prompt]\nshow status"));
        assert_ne!(result.stdout, "-p show status");
    }

    #[test]
    fn run_backend_emits_assembled_prompt_in_hook_context() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        runtime
            .run_backend(&BackendRunRequest {
                prompt: "show status".to_string(),
                cwd: PathBuf::from("/tmp"),
                backend: BackendKind::OfficialCli,
            })
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

    #[test]
    fn run_backend_forwards_hooks_to_adapter_in_order() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();

        let adapter = ForwardingHookAdapter::default();
        let events = adapter.events.clone();
        let runtime = RuntimeBootstrap::with_hook_adapter(config, adapter);
        let result = runtime
            .run_backend(&BackendRunRequest {
                prompt: "headless adapter".to_string(),
                cwd: PathBuf::from("/tmp"),
                backend: BackendKind::OfficialCli,
            })
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

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let error = runtime
            .run_backend(&BackendRunRequest {
                prompt: "headless fail".to_string(),
                cwd: PathBuf::from("/tmp"),
                backend: BackendKind::OfficialCli,
            })
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

    #[test]
    fn run_backend_emits_failure_hooks_for_nonzero_exit() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/usr/bin/false".to_string();

        let runtime = RuntimeBootstrap::with_hook_shell(config, HookShell::recording());
        let result = runtime
            .run_backend(&BackendRunRequest {
                prompt: "headless fail".to_string(),
                cwd: PathBuf::from("/tmp"),
                backend: BackendKind::OfficialCli,
            })
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
    fn unsupported_run_backend_does_not_emit_hooks() {
        let runtime =
            RuntimeBootstrap::with_hook_shell(VolvaConfig::default(), HookShell::recording());

        let error = runtime
            .run_backend(&BackendRunRequest {
                prompt: "headless fail".to_string(),
                cwd: PathBuf::from("/tmp"),
                backend: BackendKind::AnthropicApi,
            })
            .expect_err("unsupported backend should fail before hook dispatch");

        assert!(
            error
                .to_string()
                .contains("not available through `volva run` yet"),
            "unexpected error: {error}"
        );
        assert!(runtime.hook_events().is_empty());
    }
}
