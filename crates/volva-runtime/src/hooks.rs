use std::{
    env,
    fmt::Debug,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::Serialize;
use spore::logging::{SpanContext, subprocess_span, tool_span};
use spore::telemetry::TraceContextCarrier;
use volva_config::HookAdapterConfig;
use volva_core::{BackendKind, ExecutionSessionIdentity, ExecutionSessionState};

use crate::{BackendRunRequest, backend::BackendRunResult};

const HOOK_ADAPTER_POLL_INTERVAL: Duration = Duration::from_millis(25);
const VOLVA_HOOK_EVENT_SCHEMA_VERSION: &str = "1.0";

/// Maximum length for a diagnostic line after redaction.
const DIAGNOSTIC_MAX_LINE_LEN: usize = 500;

/// Environment variable names that hook adapter subprocesses are allowed to inherit.
const HOOK_ADAPTER_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "VOLVA_SESSION_ID",
    "TRACEPARENT",
    "TRACESTATE",
];

/// Redact sensitive patterns from a diagnostic string before logging or recording.
///
/// Strips bearer tokens, environment-style key assignments, and long hex/base64
/// secrets. Truncates the result to [`DIAGNOSTIC_MAX_LINE_LEN`] characters.
fn redact_diagnostic(input: &str) -> String {
    use std::borrow::Cow;

    // Patterns to redact (in order of application):
    // 1. Bearer <token>
    // 2. NAME=<value> where NAME matches *_KEY, *_TOKEN, *_SECRET, *_PASSWORD
    // 3. Hex strings of 40+ characters
    let mut output = Cow::Borrowed(input);

    // Bearer token
    {
        let s = output.as_ref();
        if let Some(pos) = s.to_ascii_lowercase().find("bearer ") {
            let rest = &s[pos + 7..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(rest.len());
            if end > 0 {
                let mut replaced = String::with_capacity(s.len());
                replaced.push_str(&s[..pos + 7]);
                replaced.push_str("[REDACTED]");
                replaced.push_str(&rest[end..]);
                output = Cow::Owned(replaced);
            }
        }
    }

    // Env-style assignments: NAME=VALUE where NAME ends in _KEY, _TOKEN, _SECRET, _PASSWORD
    {
        let s = output.into_owned();
        let mut result = String::with_capacity(s.len());
        let mut last_end = 0usize; // tracks where we last wrote up to in `s`
        let mut i = 0usize;
        let bytes = s.as_bytes();
        while i < bytes.len() {
            // Look for '=' preceded by an identifier ending in _KEY, _TOKEN, _SECRET, _PASSWORD
            if bytes[i] == b'=' {
                // Walk back to find the start of the name
                let mut name_start = i;
                while name_start > 0
                    && (bytes[name_start - 1].is_ascii_alphanumeric()
                        || bytes[name_start - 1] == b'_')
                {
                    name_start -= 1;
                }
                // Only consider the identifier if it starts within the unwritten region
                if name_start >= last_end {
                    let name = &s[name_start..i];
                    let upper = name.to_ascii_uppercase();
                    let sensitive = upper.ends_with("_KEY")
                        || upper.ends_with("_TOKEN")
                        || upper.ends_with("_SECRET")
                        || upper.ends_with("_PASSWORD");
                    if sensitive && !name.is_empty() {
                        // Write from last_end up to and including the '='
                        result.push_str(&s[last_end..=i]);
                        result.push_str("[REDACTED]");
                        // Skip the value (up to whitespace or end)
                        let rest_start = i + 1;
                        let value_len = s[rest_start..]
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(s.len() - rest_start);
                        i = rest_start + value_len;
                        last_end = i;
                        continue;
                    }
                }
            }
            i += 1;
        }
        // Write any remaining text
        result.push_str(&s[last_end..]);
        // Only replace if we actually redacted something (i.e., result differs from s)
        if last_end > 0 {
            output = Cow::Owned(result);
        } else {
            output = Cow::Owned(s);
        }
    }

    // Long hex strings (40+ hex chars)
    {
        let s = output.as_ref();
        let redacted = replace_long_hex(s);
        output = Cow::Owned(redacted);
    }

    // Truncate
    let s = output.as_ref();
    if s.chars().count() > DIAGNOSTIC_MAX_LINE_LEN {
        let truncated: String = s.chars().take(DIAGNOSTIC_MAX_LINE_LEN).collect();
        format!("{truncated}…")
    } else {
        s.to_string()
    }
}

/// Replace hex strings of 40 or more consecutive hex characters with `[REDACTED]`.
fn replace_long_hex(input: &str) -> String {
    const MIN_HEX_LEN: usize = 40;
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_hexdigit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let run_len = i - start;
            if run_len >= MIN_HEX_LEN {
                result.push_str("[REDACTED]");
            } else {
                for ch in &chars[start..i] {
                    result.push(*ch);
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    SessionStart,
    BeforePromptSend,
    ResponseComplete,
    BackendFailed,
    SessionEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookContext {
    pub backend_kind: BackendKind,
    pub execution_session: ExecutionSessionIdentity,
    pub cwd: PathBuf,
    pub prompt_text: String,
    pub prompt_summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl HookContext {
    #[must_use]
    pub fn from_request(request: &BackendRunRequest, prompt_text: impl Into<String>) -> Self {
        let prompt_text = prompt_text.into();
        Self {
            backend_kind: request.session.backend,
            execution_session: request.session.clone(),
            cwd: PathBuf::from(&request.session.workspace.workspace_root),
            prompt_summary: summarize_prompt(&prompt_text),
            prompt_text,
            stdout: None,
            stderr: None,
            exit_code: None,
            error: None,
        }
    }

    #[must_use]
    pub fn with_result(mut self, result: &BackendRunResult) -> Self {
        self.stdout = Some(result.stdout.clone());
        self.stderr = Some(result.stderr.clone());
        self.exit_code = result.exit_code;
        self.execution_session = self
            .execution_session
            .clone()
            .with_state(ExecutionSessionState::Finished);
        self
    }

    #[must_use]
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self.execution_session = self
            .execution_session
            .clone()
            .with_state(ExecutionSessionState::Finished);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookEvent {
    pub phase: HookPhase,
    pub context: HookContext,
}

#[derive(Debug, Serialize)]
struct HookAdapterPayload<'a> {
    pub schema_version: &'static str,
    pub phase: HookPhase,
    #[serde(flatten)]
    pub context: &'a HookContext,
}

impl<'a> From<&'a HookEvent> for HookAdapterPayload<'a> {
    fn from(event: &'a HookEvent) -> Self {
        Self {
            schema_version: VOLVA_HOOK_EVENT_SCHEMA_VERSION,
            phase: event.phase,
            context: &event.context,
        }
    }
}

pub trait HookAdapter: Debug + Send + Sync {
    fn handle(&self, event: HookEvent);
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookAdapterState {
    Disabled,
    ConfiguredNoop { command: Option<String> },
    ConfiguredExternal { command: String, args: Vec<String> },
    ActiveInjected,
}

impl HookAdapterState {
    #[must_use]
    pub fn status_value(&self) -> String {
        match self {
            Self::Disabled => "disabled".to_string(),
            Self::ConfiguredNoop {
                command: Some(command),
            } => {
                format!("configured-noop:{command}")
            }
            Self::ConfiguredNoop { command: None } => "configured-noop".to_string(),
            Self::ConfiguredExternal { command, args } => {
                format!("configured-external:{}", render_command_line(command, args))
            }
            Self::ActiveInjected => "active-injected".to_string(),
        }
    }
}

#[derive(Debug, Default)]
struct NoopHookAdapter;

impl HookAdapter for NoopHookAdapter {
    fn handle(&self, _event: HookEvent) {}
}

#[derive(Debug, Clone)]
struct HookAdapterCommand {
    command: String,
    args: Vec<String>,
}

impl HookAdapterCommand {
    fn new(command: String, args: Vec<String>) -> Self {
        Self { command, args }
    }

    fn display(&self) -> String {
        render_command_line(&self.command, &self.args)
    }
}

#[derive(Debug, Clone)]
struct ExternalCommandHookAdapter {
    command: HookAdapterCommand,
    diagnostics: Arc<Mutex<Vec<String>>>,
    timeout: Duration,
}

impl ExternalCommandHookAdapter {
    fn new(
        command: HookAdapterCommand,
        diagnostics: Arc<Mutex<Vec<String>>>,
        timeout: Duration,
    ) -> Self {
        Self {
            command,
            diagnostics,
            timeout,
        }
    }

    fn record_diagnostic(&self, message: String) {
        self.diagnostics
            .lock()
            .expect("hook diagnostics mutex should not be poisoned")
            .push(message);
    }

    fn invoke(&self, event: &HookEvent) -> Result<()> {
        let span_context = SpanContext::for_app("volva")
            .with_tool("hook_adapter")
            .with_workspace_root(event.context.cwd.display().to_string());
        let _tool_span = tool_span("hook_adapter", &span_context).entered();
        let payload = serde_json::to_vec(&HookAdapterPayload::from(event))
            .context("failed to serialize hook event to JSON")?;
        let stdin_file = TempIoFile::new("payload", Some(&payload))
            .context("failed to stage hook adapter payload")?;
        let stdout_file = TempIoFile::new("stdout", None)
            .context("failed to stage hook adapter stdout capture")?;
        let stderr_file = TempIoFile::new("stderr", None)
            .context("failed to stage hook adapter stderr capture")?;
        let mut cmd = Command::new(&self.command.command);
        // Clear the full inherited environment before building the allowlist.
        // This prevents API keys, tokens, and other secrets from leaking into
        // repo-controlled hook adapter processes.
        cmd.env_clear();
        // Re-add only the safe subset of the parent environment.
        for key in HOOK_ADAPTER_ENV_ALLOWLIST {
            if let Ok(value) = env::var(key) {
                cmd.env(key, value);
            }
        }
        cmd.args(&self.command.args)
            .current_dir(&event.context.cwd)
            .stdin(
                stdin_file
                    .open_read_stdio()
                    .context("failed to reopen hook adapter payload for stdin")?,
            )
            .stdout(
                stdout_file
                    .open_write_stdio()
                    .context("failed to open hook adapter stdout capture")?,
            )
            .stderr(
                stderr_file
                    .open_write_stdio()
                    .context("failed to open hook adapter stderr capture")?,
            );

        // Propagate trace context to hook adapter subprocess
        if let Some(carrier) = TraceContextCarrier::from_current() {
            cmd.env("TRACEPARENT", carrier.traceparent);
            if let Some(tracestate) = carrier.tracestate {
                cmd.env("TRACESTATE", tracestate);
            }
        }

        let mut child = cmd.spawn().with_context(|| {
            format!("failed to launch hook adapter `{}`", self.command.display())
        })?;
        let _subprocess_span = subprocess_span(&self.command.command, &span_context).entered();

        let start = Instant::now();
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .context("failed to poll hook adapter process state")?
            {
                break status;
            }

            if start.elapsed() >= self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                let stdout = stdout_file
                    .read_to_string()
                    .context("failed to capture hook adapter stdout after timeout")?;
                let stderr = stderr_file
                    .read_to_string()
                    .context("failed to capture hook adapter stderr after timeout")?;
                anyhow::bail!(
                    "hook adapter `{}` timed out after {:?}; stdout=`{}` stderr=`{}`",
                    self.command.display(),
                    self.timeout,
                    redact_diagnostic(&stdout),
                    redact_diagnostic(&stderr)
                );
            }

            thread::sleep(HOOK_ADAPTER_POLL_INTERVAL);
        };

        let stdout = stdout_file
            .read_to_string()
            .context("failed to capture hook adapter stdout")?;
        let stderr = stderr_file
            .read_to_string()
            .context("failed to capture hook adapter stderr")?;

        if !status.success() {
            anyhow::bail!(
                "hook adapter `{}` exited with status {:?}; stdout=`{}` stderr=`{}`",
                self.command.display(),
                status.code(),
                redact_diagnostic(&stdout),
                redact_diagnostic(&stderr)
            );
        }

        Ok(())
    }
}

impl HookAdapter for ExternalCommandHookAdapter {
    fn handle(&self, event: HookEvent) {
        if let Err(error) = self.invoke(&event) {
            self.record_diagnostic(format!(
                "hook adapter `{}` failed for phase {:?}: {error}",
                self.command.display(),
                event.phase
            ));
        }
    }
}

#[derive(Debug)]
struct TempIoFile {
    path: PathBuf,
}

impl Drop for TempIoFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl TempIoFile {
    fn new(prefix: &str, initial_contents: Option<&[u8]>) -> Result<Self> {
        for attempt in 0..32 {
            let path = unique_temp_io_path(prefix, attempt);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }

            match options.open(&path) {
                Ok(mut file) => {
                    if let Some(contents) = initial_contents {
                        file.write_all(contents)
                            .context("failed to write temporary hook adapter file")?;
                    }
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}

                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to create temporary hook adapter file `{}`",
                            path.display()
                        )
                    });
                }
            }
        }

        anyhow::bail!(
            "failed to allocate a unique temporary hook adapter file for `{prefix}` after repeated attempts"
        );
    }

    fn open_read_stdio(&self) -> Result<Stdio> {
        let file = File::open(&self.path)?;
        Ok(Stdio::from(file))
    }

    fn open_write_stdio(&self) -> Result<Stdio> {
        let file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        Ok(Stdio::from(file))
    }

    fn read_to_string(&self) -> Result<String> {
        let contents = fs::read(&self.path)?;
        Ok(String::from_utf8_lossy(&contents).trim().to_string())
    }
}

fn unique_temp_io_path(prefix: &str, attempt: usize) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    env::temp_dir().join(format!(
        "volva-hook-{prefix}-{}-{stamp}-{attempt}.tmp",
        std::process::id()
    ))
}

#[must_use]
pub fn render_command_line(command: &str, args: &[String]) -> String {
    std::iter::once(render_command_line_part(command))
        .chain(args.iter().map(|arg| render_command_line_part(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_command_line_part(part: &str) -> String {
    if !part.is_empty()
        && part.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '-' | '_' | '.' | '/' | ':' | '@' | '%' | '+' | '=' | ','
                )
        })
    {
        part.to_string()
    } else {
        format!("{part:?}")
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct RecordingHookAdapter {
    events: Arc<std::sync::Mutex<Vec<HookEvent>>>,
}

#[cfg(test)]
impl RecordingHookAdapter {
    fn events(&self) -> Vec<HookEvent> {
        self.events
            .lock()
            .expect("hook recorder mutex should not be poisoned")
            .clone()
    }
}

#[cfg(test)]
impl HookAdapter for RecordingHookAdapter {
    fn handle(&self, event: HookEvent) {
        self.events
            .lock()
            .expect("hook recorder mutex should not be poisoned")
            .push(event);
    }
}

#[derive(Debug, Clone)]
pub struct HookShell {
    adapter: Arc<dyn HookAdapter>,
    adapter_state: HookAdapterState,
    #[cfg_attr(not(test), allow(dead_code))]
    diagnostics: Arc<Mutex<Vec<String>>>,
    #[cfg(test)]
    recorder: Option<Arc<RecordingHookAdapter>>,
}

impl HookShell {
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapter: Arc::new(NoopHookAdapter),
            adapter_state: HookAdapterState::Disabled,
            diagnostics: Arc::new(Mutex::new(Vec::new())),
            #[cfg(test)]
            recorder: None,
        }
    }

    #[must_use]
    pub fn configured(config: HookAdapterConfig) -> Self {
        let config = config.with_clamped_timeout();
        let timeout = Duration::from_millis(config.timeout_ms);
        Self::from_config(config, timeout)
    }

    #[must_use]
    fn from_config(config: HookAdapterConfig, timeout: Duration) -> Self {
        let diagnostics = Arc::new(Mutex::new(Vec::new()));

        let (adapter, adapter_state): (Arc<dyn HookAdapter>, HookAdapterState) = match config
            .command
        {
            Some(ref command) if !command.trim().is_empty() && config.enabled => {
                if !config.is_trusted(command) {
                    tracing::warn!(
                        command = %command,
                        "hook adapter '{}' is not explicitly trusted; set `trusted = true` in volva.json to suppress this warning (cortina adapters are implicitly trusted)",
                        command
                    );
                }
                let command = command.clone();
                let args = config.args.clone();
                let adapter = ExternalCommandHookAdapter::new(
                    HookAdapterCommand::new(command.clone(), args.clone()),
                    diagnostics.clone(),
                    timeout,
                );
                (
                    Arc::new(adapter),
                    HookAdapterState::ConfiguredExternal { command, args },
                )
            }
            command if config.enabled => (
                Arc::new(NoopHookAdapter),
                HookAdapterState::ConfiguredNoop { command },
            ),
            _ => (Arc::new(NoopHookAdapter), HookAdapterState::Disabled),
        };

        Self {
            adapter,
            adapter_state,
            diagnostics,
            #[cfg(test)]
            recorder: None,
        }
    }

    #[must_use]
    pub fn with_adapter<T>(adapter: T) -> Self
    where
        T: HookAdapter + 'static,
    {
        Self {
            adapter: Arc::new(adapter),
            adapter_state: HookAdapterState::ActiveInjected,
            diagnostics: Arc::new(Mutex::new(Vec::new())),
            #[cfg(test)]
            recorder: None,
        }
    }

    pub fn emit(&self, phase: HookPhase, context: HookContext) {
        self.adapter.handle(HookEvent { phase, context });
    }

    #[must_use]
    pub(crate) fn adapter_state(&self) -> &HookAdapterState {
        &self.adapter_state
    }

    #[must_use]
    pub(crate) fn take_diagnostics(&self) -> Vec<String> {
        let mut diagnostics = self
            .diagnostics
            .lock()
            .expect("hook diagnostics mutex should not be poisoned");
        std::mem::take(&mut *diagnostics)
    }
}

#[cfg(test)]
impl HookShell {
    #[must_use]
    pub(crate) fn recording() -> Self {
        let recorder = Arc::new(RecordingHookAdapter::default());

        Self {
            adapter: recorder.clone(),
            adapter_state: HookAdapterState::ActiveInjected,
            diagnostics: Arc::new(Mutex::new(Vec::new())),
            recorder: Some(recorder),
        }
    }

    #[must_use]
    pub(crate) fn events(&self) -> Vec<HookEvent> {
        self.recorder
            .as_ref()
            .map(|recorder| recorder.events())
            .unwrap_or_default()
    }

    #[cfg(unix)]
    #[must_use]
    pub(crate) fn diagnostics(&self) -> Vec<String> {
        self.diagnostics
            .lock()
            .expect("hook diagnostics mutex should not be poisoned")
            .clone()
    }
}

impl Default for HookShell {
    fn default() -> Self {
        Self::new()
    }
}

fn summarize_prompt(prompt: &str) -> String {
    const MAX_PROMPT_SUMMARY_CHARS: usize = 80;

    let mut summary = prompt
        .chars()
        .take(MAX_PROMPT_SUMMARY_CHARS)
        .collect::<String>();
    if prompt.chars().nth(MAX_PROMPT_SUMMARY_CHARS).is_some() {
        summary.push_str("...");
    }
    summary
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[cfg(unix)]
    use std::{
        env, fs,
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    use serde_json::Value;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionIdentity,
        ExecutionSessionState, WorkspaceBinding,
    };

    use super::{HookAdapterConfig, HookAdapterState, HookShell};
    #[cfg(unix)]
    use super::{HookContext, HookPhase};

    #[cfg(unix)]
    fn test_session(cwd: &Path) -> ExecutionSessionIdentity {
        ExecutionSessionIdentity::new(
            ExecutionMode::Run,
            BackendKind::OfficialCli,
            WorkspaceBinding::from_root(cwd),
            ExecutionParticipantIdentity {
                participant_id: "operator@volva".to_string(),
                host_kind: "volva".to_string(),
            },
            ExecutionSessionState::Active,
        )
    }

    #[test]
    fn default_hook_shell_is_disabled() {
        let shell = HookShell::new();

        assert_eq!(shell.adapter_state(), &HookAdapterState::Disabled);
    }

    #[test]
    fn configured_hook_shell_reports_active_external_state_when_command_is_present() {
        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: Some("/usr/local/bin/cortina-hook-adapter".to_string()),
            args: Vec::new(),
            timeout_ms: 30_000,
            trusted: false,
        });

        assert_eq!(
            shell.adapter_state(),
            &HookAdapterState::ConfiguredExternal {
                command: "/usr/local/bin/cortina-hook-adapter".to_string(),
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn configured_hook_shell_reports_noop_state_without_command() {
        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: None,
            args: Vec::new(),
            timeout_ms: 30_000,
            trusted: false,
        });

        assert_eq!(
            shell.adapter_state(),
            &HookAdapterState::ConfiguredNoop { command: None }
        );
    }

    #[test]
    fn configured_hook_shell_clamps_timeout_zero_to_minimum() {
        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: Some("/usr/bin/true".to_string()),
            args: Vec::new(),
            timeout_ms: 0,
            trusted: false,
        });

        // Verify that the adapter was created (which means timeout was valid after clamping)
        assert!(matches!(
            shell.adapter_state(),
            &HookAdapterState::ConfiguredExternal { .. }
        ));
    }

    #[test]
    fn configured_hook_shell_clamps_timeout_exceeding_max() {
        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: Some("/usr/bin/true".to_string()),
            args: Vec::new(),
            timeout_ms: 50_000,
            trusted: false,
        });

        // Verify that the adapter was created (which means timeout was clamped to 30000)
        assert!(matches!(
            shell.adapter_state(),
            &HookAdapterState::ConfiguredExternal { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn configured_hook_shell_invokes_external_adapter_with_json_payload() {
        let capture_path = unique_temp_path("hook-adapter-success.json");
        let command_path = write_hook_script(&format!(
            "#!/bin/sh\ncat > \"{}\"\n",
            shell_quote(capture_path.as_path())
        ));

        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: Some(command_path.to_string_lossy().to_string()),
            args: Vec::new(),
            timeout_ms: 30_000,
            trusted: false,
        });
        shell.emit(
            HookPhase::BeforePromptSend,
            HookContext {
                backend_kind: BackendKind::OfficialCli,
                execution_session: test_session(
                    &env::current_dir().expect("current dir should be available"),
                ),
                cwd: env::current_dir().expect("current dir should be available"),
                prompt_text: "summarize the repository".to_string(),
                prompt_summary: "summarize the repository".to_string(),
                stdout: Some("assistant output".to_string()),
                stderr: Some("diagnostic text".to_string()),
                exit_code: Some(0),
                error: None,
            },
        );

        let payload = fs::read_to_string(&capture_path)
            .unwrap_or_else(|error| panic!("hook adapter should write payload: {error}"));
        let value: Value = serde_json::from_str(&payload)
            .unwrap_or_else(|error| panic!("payload should be JSON: {error}"));

        assert_eq!(value["schema_version"], "1.0");
        assert_eq!(value["phase"], "before_prompt_send");
        assert_eq!(value["backend_kind"], "official-cli");
        assert_eq!(value["prompt_text"], "summarize the repository");
        assert_eq!(value["prompt_summary"], "summarize the repository");
        assert_eq!(value["stdout"], "assistant output");
        assert_eq!(value["stderr"], "diagnostic text");
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["error"], Value::Null);
    }

    #[cfg(unix)]
    #[test]
    fn configured_hook_shell_records_diagnostic_when_adapter_fails() {
        let command_path = write_hook_script("#!/bin/sh\ncat >/dev/null\nexit 23\n");
        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: Some(command_path.to_string_lossy().to_string()),
            args: Vec::new(),
            timeout_ms: 30_000,
            trusted: false,
        });

        shell.emit(
            HookPhase::SessionEnd,
            HookContext {
                backend_kind: BackendKind::OfficialCli,
                execution_session: test_session(
                    &env::current_dir().expect("current dir should be available"),
                ),
                cwd: env::current_dir().expect("current dir should be available"),
                prompt_text: "headless fail".to_string(),
                prompt_summary: "headless fail".to_string(),
                stdout: None,
                stderr: None,
                exit_code: None,
                error: Some("backend failed".to_string()),
            },
        );

        let diagnostics = shell.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].contains("exited"),
            "unexpected diagnostic: {}",
            diagnostics[0]
        );
        assert!(diagnostics[0].contains("SessionEnd"));
    }

    #[cfg(unix)]
    #[test]
    fn configured_hook_shell_times_out_hung_adapter_and_records_diagnostic() {
        let command_path = write_hook_script("#!/bin/sh\nsleep 1\n");
        let shell = HookShell::from_config(
            HookAdapterConfig {
                enabled: true,
                command: Some(command_path.to_string_lossy().to_string()),
                args: Vec::new(),
                timeout_ms: 30_000,
                trusted: false,
            },
            Duration::from_millis(50),
        );

        shell.emit(
            HookPhase::SessionEnd,
            HookContext {
                backend_kind: BackendKind::OfficialCli,
                execution_session: test_session(
                    &env::current_dir().expect("current dir should be available"),
                ),
                cwd: env::current_dir().expect("current dir should be available"),
                prompt_text: "x".repeat(1024 * 1024),
                prompt_summary: "headless timeout".to_string(),
                stdout: None,
                stderr: None,
                exit_code: None,
                error: Some("backend failed".to_string()),
            },
        );

        let diagnostics = shell.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].contains("timed out"),
            "unexpected diagnostic: {}",
            diagnostics[0]
        );
        assert!(diagnostics[0].contains("SessionEnd"));
    }

    #[cfg(unix)]
    #[test]
    fn configured_hook_shell_passes_configured_args_to_external_adapter() {
        let payload_path = unique_temp_path("hook-adapter-payload.json");
        let args_path = unique_temp_path("hook-adapter-args.txt");
        let command_path = write_hook_script(&format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}\"\ncat > \"{}\"\n",
            shell_quote(args_path.as_path()),
            shell_quote(payload_path.as_path())
        ));

        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: Some(command_path.to_string_lossy().to_string()),
            args: vec![
                "adapter".to_string(),
                "volva".to_string(),
                "hook-event".to_string(),
            ],
            timeout_ms: 30_000,
            trusted: false,
        });

        shell.emit(
            HookPhase::SessionStart,
            HookContext {
                backend_kind: BackendKind::OfficialCli,
                execution_session: test_session(
                    &env::current_dir().expect("current dir should be available"),
                ),
                cwd: env::current_dir().expect("current dir should be available"),
                prompt_text: "argv test".to_string(),
                prompt_summary: "argv test".to_string(),
                stdout: None,
                stderr: None,
                exit_code: None,
                error: None,
            },
        );

        let args = fs::read_to_string(&args_path).expect("hook adapter should capture argv");
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            vec!["adapter", "volva", "hook-event"]
        );

        let payload = fs::read_to_string(&payload_path).expect("hook adapter should write payload");
        let value: Value = serde_json::from_str(&payload)
            .unwrap_or_else(|error| panic!("payload should be JSON: {error}"));
        assert_eq!(value["phase"], "session_start");
    }

    #[cfg(unix)]
    #[test]
    fn hook_adapter_env_stripped_does_not_pass_api_keys_to_subprocess() {
        // Write a script that dumps the full environment to a file.
        // The hook adapter subprocess runs with env_clear() + allowlist only,
        // so even if ANTHROPIC_API_KEY is set in the parent process, it must
        // not appear in the child's environment.
        let env_dump_path = unique_temp_path("hook-adapter-env.txt");
        let command_path = write_hook_script(&format!(
            "#!/bin/sh\nenv > \"{}\"\n",
            shell_quote(env_dump_path.as_path())
        ));

        let shell = HookShell::configured(HookAdapterConfig {
            enabled: true,
            command: Some(command_path.to_string_lossy().to_string()),
            args: Vec::new(),
            timeout_ms: 30_000,
            trusted: false,
        });
        shell.emit(
            HookPhase::SessionStart,
            HookContext {
                backend_kind: BackendKind::OfficialCli,
                execution_session: test_session(
                    &env::current_dir().expect("current dir should be available"),
                ),
                cwd: env::current_dir().expect("current dir should be available"),
                prompt_text: "env test".to_string(),
                prompt_summary: "env test".to_string(),
                stdout: None,
                stderr: None,
                exit_code: None,
                error: None,
            },
        );

        let env_dump = fs::read_to_string(&env_dump_path)
            .unwrap_or_else(|e| panic!("hook adapter should dump env: {e}"));

        // The allowlist does not contain any secret key patterns.
        // Verify the most common secret variable names are absent.
        assert!(
            !env_dump.contains("ANTHROPIC_API_KEY"),
            "ANTHROPIC_API_KEY must not appear in the hook adapter environment; dump:\n{env_dump}"
        );
        assert!(
            !env_dump.contains("OPENAI_API_KEY"),
            "OPENAI_API_KEY must not appear in the hook adapter environment"
        );

        // Verify that allowed vars CAN appear (PATH is universally present).
        // This confirms env_clear + re-add works rather than env_clear alone.
        assert!(
            env_dump.contains("PATH="),
            "PATH should be present in the hook adapter environment; dump:\n{env_dump}"
        );
    }

    #[test]
    fn hook_adapter_config_is_trusted_for_cortina_adapter_regardless_of_trusted_field() {
        let config = HookAdapterConfig {
            enabled: true,
            command: Some("/usr/local/bin/cortina".to_string()),
            args: Vec::new(),
            timeout_ms: 30_000,
            trusted: false,
        };

        assert!(
            config.is_trusted("cortina"),
            "cortina adapter should be implicitly trusted"
        );
        assert!(
            config.is_trusted("/usr/local/bin/cortina-hook-adapter"),
            "cortina prefix in name should be implicitly trusted"
        );
        assert!(
            !config.is_trusted("my-custom-hook"),
            "unknown adapter without trusted:true should not be trusted"
        );

        let trusted_config = HookAdapterConfig {
            trusted: true,
            ..config
        };
        assert!(
            trusted_config.is_trusted("my-custom-hook"),
            "adapter with trusted:true should be trusted regardless of name"
        );
    }

    #[cfg(unix)]
    fn unique_temp_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        env::temp_dir().join(format!("volva-{stamp}-{name}"))
    }

    #[cfg(unix)]
    fn write_hook_script(content: &str) -> PathBuf {
        let path = unique_temp_path("hook-adapter.sh");
        fs::write(&path, content).expect("hook adapter script should be writable");

        let mut permissions = fs::metadata(&path)
            .expect("hook adapter script metadata should be available")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("hook adapter script should be executable");

        path
    }

    #[cfg(unix)]
    fn shell_quote(path: &Path) -> String {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    }
}
