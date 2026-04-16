use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use clap::{Args, Subcommand, ValueEnum};
use serde::Deserialize;
use spore::logging::{SpanContext, subprocess_span, tool_span};

use volva_config::VolvaConfig;
use volva_core::BackendKind;
use volva_runtime::{
    BackendSessionSurface, RuntimeBootstrap, render_command_line, session_status_lines,
};

#[derive(Debug, Args)]
pub struct BackendCommand {
    #[command(subcommand)]
    pub command: BackendSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum BackendSubcommand {
    Status(StatusSubcommand),
    Doctor(DoctorSubcommand),
    Session(SessionSubcommand),
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct StatusSubcommand {}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct DoctorSubcommand {}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct SessionSubcommand {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendArg {
    OfficialCli,
    AnthropicApi,
}

impl From<BackendArg> for BackendKind {
    fn from(value: BackendArg) -> Self {
        match value {
            BackendArg::OfficialCli => Self::OfficialCli,
            BackendArg::AnthropicApi => Self::AnthropicApi,
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
pub fn handle_backend(command: BackendCommand) -> Result<()> {
    match command.command {
        BackendSubcommand::Status(StatusSubcommand {}) => {
            let root = env::current_dir()?;
            let config = VolvaConfig::load_from(&root)?;
            let runtime = RuntimeBootstrap::new(config);
            for line in render_backend_status(&runtime) {
                println!("{line}");
            }
            Ok(())
        }
        BackendSubcommand::Doctor(DoctorSubcommand {}) => {
            let root = env::current_dir()?;
            let config = VolvaConfig::load_from(&root)?;
            let runtime = RuntimeBootstrap::new(config);
            for line in render_backend_doctor(&runtime, &root) {
                println!("{line}");
            }
            Ok(())
        }
        BackendSubcommand::Session(SessionSubcommand { json }) => {
            let root = env::current_dir()?;
            let config = VolvaConfig::load_from(&root)?;
            let runtime = RuntimeBootstrap::new(config);
            let surface = runtime.load_execution_session()?.ok_or_else(|| {
                anyhow!(
                    "no persisted execution session is available yet; run `volva run` or `volva chat` first"
                )
            })?;

            if json {
                println!("{}", serde_json::to_string_pretty(&surface)?);
                return Ok(());
            }

            for line in render_backend_session(&surface) {
                println!("{line}");
            }
            Ok(())
        }
    }
}

fn render_backend_status(runtime: &RuntimeBootstrap) -> Vec<String> {
    let status = runtime.backend_status();
    let mut lines = vec![
        format!("backend: {}", status.kind),
        format!("command: {}", status.command),
    ];

    if let Some(hook_adapter) = runtime
        .status_lines()
        .into_iter()
        .find(|line| line.label == "hook_adapter")
        .map(|line| line.value)
    {
        lines.push(format!("hook_adapter: {hook_adapter}"));
    }

    lines
}

pub(crate) fn render_backend_doctor(runtime: &RuntimeBootstrap, cwd: &Path) -> Vec<String> {
    let backend_status = runtime.backend_status();
    let backend_supported_by_run = backend_supported_by_run(backend_status.kind);
    let backend_command_resolved = command_resolved(&backend_status.command);
    let hook_adapter_state = runtime
        .status_lines()
        .into_iter()
        .find(|line| line.label == "hook_adapter")
        .map_or_else(|| "unknown".to_string(), |line| line.value);
    let hook_adapter_command = hook_adapter_command_line(runtime);
    let hook_adapter_command_resolved = if runtime.config.hook_adapter.enabled {
        runtime
            .config
            .hook_adapter
            .command
            .as_deref()
            .is_some_and(command_resolved)
    } else {
        true
    };
    let local_backend_ready =
        backend_supported_by_run && backend_command_resolved && hook_adapter_command_resolved;
    let hook_delivery_health = collect_hook_delivery_health(runtime, cwd);
    let hook_delivery_ready = hook_delivery_health.ready_for_supported_path();
    let backend_ready = local_backend_ready && hook_delivery_ready.unwrap_or(true);

    let mut lines = vec![
        format!("local_backend_ready: {local_backend_ready}"),
        format!("backend_ready: {backend_ready}"),
        format!("backend: {}", backend_status.kind),
        format!("backend_supported_by_run: {backend_supported_by_run}"),
        format!("backend_command: {}", backend_status.command),
        format!("backend_command_resolved: {backend_command_resolved}"),
        format!("hook_adapter: {hook_adapter_state}"),
        format!("hook_adapter_command_line: {hook_adapter_command}"),
        format!("hook_adapter_command_resolved: {hook_adapter_command_resolved}"),
        format!(
            "hook_adapter_timeout_ms: {}",
            runtime.config.hook_adapter.timeout_ms
        ),
    ];
    lines.extend(hook_delivery_health.render_lines());
    lines
}

pub(crate) fn render_backend_session(surface: &BackendSessionSurface) -> Vec<String> {
    let mut lines = vec![
        format!("backend: {}", surface.backend),
        format!("backend_command: {}", surface.backend_command),
        format!("run_supported: {}", surface.run_supported),
    ];

    lines.extend(
        session_status_lines(&surface.session)
            .into_iter()
            .map(|line| format!("{}: {}", line.label, line.value)),
    );

    lines
}

pub(crate) fn command_resolved(command: &str) -> bool {
    let candidate = Path::new(command);
    if candidate.is_absolute()
        || candidate
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        return command_candidate_paths(candidate)
            .into_iter()
            .any(|path| command_launchable(&path));
    }

    env::var_os("PATH").is_some_and(|path_var| {
        env::split_paths(&path_var)
            .map(|dir| dir.join(command))
            .flat_map(|path| command_candidate_paths(&path))
            .any(|path| command_launchable(&path))
    })
}

fn hook_adapter_command_line(runtime: &RuntimeBootstrap) -> String {
    if !runtime.config.hook_adapter.enabled {
        return "disabled".to_string();
    }

    match runtime.config.hook_adapter.command.as_deref() {
        Some(command) if !command.trim().is_empty() => {
            render_command_line(command, &runtime.config.hook_adapter.args)
        }
        _ => "missing".to_string(),
    }
}

fn backend_supported_by_run(kind: BackendKind) -> bool {
    matches!(kind, BackendKind::OfficialCli)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HookDeliveryHealth {
    probe: HookDeliveryProbe,
    seen_for_cwd: Option<bool>,
    event_count: Option<usize>,
    events_valid_json: Option<bool>,
    detail: Option<String>,
}

impl HookDeliveryHealth {
    fn ready_for_supported_path(&self) -> Option<bool> {
        match self.probe {
            HookDeliveryProbe::Disabled | HookDeliveryProbe::UnsupportedAdapter => None,
            HookDeliveryProbe::CommandUnresolved | HookDeliveryProbe::CortinaProbeFailed => {
                Some(false)
            }
            HookDeliveryProbe::CortinaOk => Some(self.events_valid_json.unwrap_or(false)),
        }
    }

    fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("hook_delivery_probe: {}", self.probe.as_str())];

        lines.push(match self.ready_for_supported_path() {
            Some(ready) => format!("hook_delivery_ready: {ready}"),
            None => "hook_delivery_ready: unknown".to_string(),
        });
        lines.push(match self.seen_for_cwd {
            Some(seen) => format!("hook_delivery_seen_for_cwd: {seen}"),
            None => "hook_delivery_seen_for_cwd: unknown".to_string(),
        });
        lines.push(match self.event_count {
            Some(count) => format!("hook_delivery_event_count: {count}"),
            None => "hook_delivery_event_count: unknown".to_string(),
        });
        lines.push(match self.events_valid_json {
            Some(valid) => format!("hook_delivery_events_valid_json: {valid}"),
            None => "hook_delivery_events_valid_json: unknown".to_string(),
        });
        if let Some(detail) = &self.detail {
            lines.push(format!("hook_delivery_detail: {detail}"));
        }

        lines
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookDeliveryProbe {
    Disabled,
    UnsupportedAdapter,
    CommandUnresolved,
    CortinaOk,
    CortinaProbeFailed,
}

impl HookDeliveryProbe {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::UnsupportedAdapter => "unsupported-hook-adapter",
            Self::CommandUnresolved => "cortina-command-unresolved",
            Self::CortinaOk => "cortina-status-doctor",
            Self::CortinaProbeFailed => "cortina-probe-failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CortinaProbeCommand {
    command: String,
    prefix_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CortinaStatusReport {
    volva_hook_event_count: usize,
}

#[derive(Debug, Deserialize)]
struct CortinaDoctorReport {
    volva_hook_events: CortinaFileHealth,
}

#[derive(Debug, Deserialize)]
struct CortinaFileHealth {
    valid_json: bool,
}

fn collect_hook_delivery_health(runtime: &RuntimeBootstrap, cwd: &Path) -> HookDeliveryHealth {
    let _tool_span =
        tool_span("collect_hook_delivery_health", &span_context_for_cwd(cwd)).entered();
    if !runtime.config.hook_adapter.enabled {
        return HookDeliveryHealth {
            probe: HookDeliveryProbe::Disabled,
            seen_for_cwd: None,
            event_count: None,
            events_valid_json: None,
            detail: Some("hook adapter is disabled in volva config".to_string()),
        };
    }

    let Some(probe_command) = cortina_probe_command(runtime) else {
        return HookDeliveryHealth {
            probe: HookDeliveryProbe::UnsupportedAdapter,
            seen_for_cwd: None,
            event_count: None,
            events_valid_json: None,
            detail: Some(
                "configured hook adapter is not the supported cortina adapter volva hook-event surface"
                    .to_string(),
            ),
        };
    };

    if !command_resolved(&probe_command.command) {
        return HookDeliveryHealth {
            probe: HookDeliveryProbe::CommandUnresolved,
            seen_for_cwd: None,
            event_count: None,
            events_valid_json: None,
            detail: Some(format!(
                "cannot launch `{}` for cortina status/doctor probes",
                probe_command.command
            )),
        };
    }

    let status_args = cortina_probe_args(&probe_command.prefix_args, "status", cwd);
    let doctor_args = cortina_probe_args(&probe_command.prefix_args, "doctor", cwd);
    let timeout = Duration::from_millis(runtime.config.hook_adapter.timeout_ms);
    let status_probe = match run_cortina_probe::<CortinaStatusReport>(
        &probe_command.command,
        &status_args,
        timeout,
    ) {
        Ok(report) => report,
        Err(error) => {
            return HookDeliveryHealth {
                probe: HookDeliveryProbe::CortinaProbeFailed,
                seen_for_cwd: None,
                event_count: None,
                events_valid_json: None,
                detail: Some(format!("status probe failed: {error}")),
            };
        }
    };
    let doctor_probe = match run_cortina_probe::<CortinaDoctorReport>(
        &probe_command.command,
        &doctor_args,
        timeout,
    ) {
        Ok(report) => report,
        Err(error) => {
            return HookDeliveryHealth {
                probe: HookDeliveryProbe::CortinaProbeFailed,
                seen_for_cwd: None,
                event_count: None,
                events_valid_json: None,
                detail: Some(format!("doctor probe failed: {error}")),
            };
        }
    };

    HookDeliveryHealth {
        probe: HookDeliveryProbe::CortinaOk,
        seen_for_cwd: Some(status_probe.volva_hook_event_count > 0),
        event_count: Some(status_probe.volva_hook_event_count),
        events_valid_json: Some(doctor_probe.volva_hook_events.valid_json),
        detail: Some("observed through cortina status/doctor --json --cwd".to_string()),
    }
}

fn cortina_probe_command(runtime: &RuntimeBootstrap) -> Option<CortinaProbeCommand> {
    let command = runtime.config.hook_adapter.command.as_deref()?.trim();
    if command.is_empty() {
        return None;
    }

    let args = &runtime.config.hook_adapter.args;
    if args.len() < 3
        || args[args.len() - 3..]
            != [
                "adapter".to_string(),
                "volva".to_string(),
                "hook-event".to_string(),
            ]
    {
        return None;
    }
    let prefix_args = args[..args.len() - 3].to_vec();

    Some(CortinaProbeCommand {
        command: command.to_string(),
        prefix_args,
    })
}

fn cortina_probe_args(prefix_args: &[String], subcommand: &str, cwd: &Path) -> Vec<String> {
    let mut args = prefix_args.to_vec();
    args.push(subcommand.to_string());
    args.push("--json".to_string());
    args.push("--cwd".to_string());
    args.push(cwd.display().to_string());
    args
}

fn run_cortina_probe<T>(command: &str, args: &[String], timeout: Duration) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let _tool_span = tool_span("cortina_probe", &span_context_for_command(command)).entered();
    let output = run_command_with_timeout(command, args, timeout)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        anyhow::bail!(
            "`{}` exited with {:?}: {}",
            render_command_line(command, args),
            output.status.code(),
            if detail.is_empty() {
                "no output".to_string()
            } else {
                detail
            }
        );
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

struct TimedCommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_command_with_timeout(
    command: &str,
    args: &[String],
    timeout: Duration,
) -> Result<TimedCommandOutput> {
    let _subprocess_span = subprocess_span(command, &span_context_for_command(command)).entered();
    let mut child = Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let status = child.wait()?;
            let stdout = read_child_stream(child.stdout.take())?;
            let stderr = read_child_stream(child.stderr.take())?;
            anyhow::bail!(
                "`{}` timed out after {:?}; status={:?}; stdout=`{}` stderr=`{}`",
                render_command_line(command, args),
                timeout,
                status.code(),
                String::from_utf8_lossy(&stdout).trim(),
                String::from_utf8_lossy(&stderr).trim()
            );
        }

        thread::sleep(Duration::from_millis(25));
    };

    Ok(TimedCommandOutput {
        status,
        stdout: read_child_stream(child.stdout.take())?,
        stderr: read_child_stream(child.stderr.take())?,
    })
}

fn span_context_for_cwd(cwd: &Path) -> SpanContext {
    SpanContext::for_app("volva")
        .with_tool("backend_doctor")
        .with_workspace_root(cwd.display().to_string())
}

fn span_context_for_command(command: &str) -> SpanContext {
    let context = SpanContext::for_app("volva").with_tool(command.to_string());
    match env::current_dir() {
        Ok(path) => context.with_workspace_root(path.display().to_string()),
        Err(_) => context,
    }
}

fn read_child_stream(stream: Option<impl std::io::Read>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut stream) = stream {
        std::io::Read::read_to_end(&mut stream, &mut bytes)?;
    }
    Ok(bytes)
}

fn command_candidate_paths(path: &Path) -> Vec<PathBuf> {
    let candidates = vec![path.to_path_buf()];

    #[cfg(windows)]
    {
        let mut candidates = candidates;
        if path.extension().is_none() {
            candidates.extend(windows_pathext_suffixes().into_iter().map(|suffix| {
                let mut candidate = path.as_os_str().to_os_string();
                candidate.push(&suffix);
                PathBuf::from(candidate)
            }));
        }
        return candidates;
    }

    #[cfg(not(windows))]
    candidates
}

#[cfg(windows)]
fn windows_pathext_suffixes() -> Vec<String> {
    const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

    env::var("PATHEXT")
        .ok()
        .as_deref()
        .unwrap_or(DEFAULT_PATHEXT)
        .split(';')
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                None
            } else if trimmed.starts_with('.') {
                Some(trimmed.to_ascii_lowercase())
            } else {
                Some(format!(".{}", trimmed.to_ascii_lowercase()))
            }
        })
        .collect()
}

#[cfg(unix)]
fn command_launchable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn command_launchable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::{
        command_resolved, cortina_probe_args, cortina_probe_command, render_backend_doctor,
        render_backend_status,
    };
    use volva_config::VolvaConfig;
    use volva_core::BackendKind;
    use volva_runtime::RuntimeBootstrap;

    #[test]
    fn backend_status_includes_hook_adapter_configuration() {
        let mut config = VolvaConfig::default();
        config.hook_adapter.enabled = true;
        config.hook_adapter.command = Some("cortina".to_string());
        config.hook_adapter.args = vec![
            "adapter".to_string(),
            "volva".to_string(),
            "hook-event".to_string(),
        ];

        let lines = render_backend_status(&RuntimeBootstrap::new(config));

        assert!(lines.contains(&"backend: official-cli".to_string()));
        assert!(lines.contains(&"command: claude".to_string()));
        assert!(lines.contains(
            &"hook_adapter: configured-external:cortina adapter volva hook-event".to_string()
        ));
    }

    #[test]
    fn backend_doctor_reports_readiness_and_resolution() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.hook_adapter.enabled = true;
        config.hook_adapter.command = Some("/bin/echo".to_string());
        config.hook_adapter.args = vec![
            "adapter".to_string(),
            "volva".to_string(),
            "hook-event".to_string(),
        ];
        config.hook_adapter.timeout_ms = 45_000;

        let lines = render_backend_doctor(&RuntimeBootstrap::new(config), Path::new("/tmp"));

        assert!(lines.contains(&"local_backend_ready: true".to_string()));
        assert!(lines.contains(&"backend_ready: false".to_string()));
        assert!(lines.contains(&"backend_supported_by_run: true".to_string()));
        assert!(lines.contains(&"backend_command: /bin/echo".to_string()));
        assert!(lines.contains(&"backend_command_resolved: true".to_string()));
        assert!(lines.contains(
            &"hook_adapter_command_line: /bin/echo adapter volva hook-event".to_string()
        ));
        assert!(lines.contains(&"hook_adapter_command_resolved: true".to_string()));
        assert!(lines.contains(&"hook_adapter_timeout_ms: 45000".to_string()));
        assert!(lines.contains(&"hook_delivery_probe: cortina-probe-failed".to_string()));
        assert!(lines.contains(&"hook_delivery_ready: false".to_string()));
    }

    #[test]
    fn backend_doctor_reports_missing_hook_adapter_command_when_enabled() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.hook_adapter.enabled = true;

        let lines = render_backend_doctor(&RuntimeBootstrap::new(config), Path::new("/tmp"));

        assert!(lines.contains(&"local_backend_ready: false".to_string()));
        assert!(lines.contains(&"backend_ready: false".to_string()));
        assert!(lines.contains(&"hook_adapter_command_line: missing".to_string()));
        assert!(lines.contains(&"hook_adapter_command_resolved: false".to_string()));
        assert!(lines.contains(&"hook_delivery_probe: unsupported-hook-adapter".to_string()));
        assert!(lines.contains(&"hook_delivery_ready: unknown".to_string()));
    }

    #[test]
    fn backend_doctor_reports_unsupported_backend_as_not_ready() {
        let mut config = VolvaConfig::default();
        config.backend.kind = BackendKind::AnthropicApi;
        config.backend.command = "/bin/echo".to_string();

        let lines = render_backend_doctor(&RuntimeBootstrap::new(config), Path::new("/tmp"));

        assert!(lines.contains(&"local_backend_ready: false".to_string()));
        assert!(lines.contains(&"backend_ready: false".to_string()));
        assert!(lines.contains(&"backend_supported_by_run: false".to_string()));
        assert!(lines.contains(&"backend_command_resolved: true".to_string()));
        assert!(lines.contains(&"hook_delivery_probe: disabled".to_string()));
        assert!(lines.contains(&"hook_delivery_ready: unknown".to_string()));
    }

    #[test]
    fn backend_doctor_renders_hook_adapter_argv_with_runtime_quoting() {
        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.hook_adapter.enabled = true;
        config.hook_adapter.command = Some("/bin/echo".to_string());
        config.hook_adapter.args = vec!["hook event".to_string()];

        let lines = render_backend_doctor(&RuntimeBootstrap::new(config), Path::new("/tmp"));

        assert!(lines.contains(&"hook_adapter_command_line: /bin/echo \"hook event\"".to_string()));
    }

    #[test]
    fn cortina_probe_command_derives_status_prefix_from_adapter_invocation() {
        let mut config = VolvaConfig::default();
        config.hook_adapter.enabled = true;
        config.hook_adapter.command = Some("cargo".to_string());
        config.hook_adapter.args = vec![
            "run".to_string(),
            "--manifest-path".to_string(),
            "/tmp/cortina/Cargo.toml".to_string(),
            "--".to_string(),
            "adapter".to_string(),
            "volva".to_string(),
            "hook-event".to_string(),
        ];

        let probe = cortina_probe_command(&RuntimeBootstrap::new(config))
            .expect("cortina adapter command should derive");
        let status_args =
            cortina_probe_args(&probe.prefix_args, "status", Path::new("/tmp/volva-smoke"));

        assert_eq!(probe.command, "cargo");
        assert_eq!(
            status_args,
            vec![
                "run".to_string(),
                "--manifest-path".to_string(),
                "/tmp/cortina/Cargo.toml".to_string(),
                "--".to_string(),
                "status".to_string(),
                "--json".to_string(),
                "--cwd".to_string(),
                "/tmp/volva-smoke".to_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn backend_doctor_reports_observed_hook_delivery_from_cortina_json_surfaces() {
        let script = unique_temp_path("fake-cortina.sh");
        fs::write(
            &script,
            "#!/bin/sh\nif [ \"$1\" = \"status\" ]; then\n  printf '%s' '{\"volva_hook_event_count\":4}'\nelif [ \"$1\" = \"doctor\" ]; then\n  printf '%s' '{\"volva_hook_events\":{\"valid_json\":true}}'\nelse\n  printf '%s' \"unexpected:$1\" >&2\n  exit 1\nfi\n",
        )
        .expect("fake cortina script should be writable");

        let mut permissions = fs::metadata(&script)
            .expect("fake cortina metadata should be available")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("fake cortina should be executable");

        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.hook_adapter.enabled = true;
        config.hook_adapter.command = Some(script.to_string_lossy().to_string());
        config.hook_adapter.args = vec![
            "adapter".to_string(),
            "volva".to_string(),
            "hook-event".to_string(),
        ];

        let lines = render_backend_doctor(
            &RuntimeBootstrap::new(config),
            Path::new("/tmp/volva-hook-health"),
        );

        assert!(lines.contains(&"local_backend_ready: true".to_string()));
        assert!(lines.contains(&"backend_ready: true".to_string()));
        assert!(lines.contains(&"hook_delivery_probe: cortina-status-doctor".to_string()));
        assert!(lines.contains(&"hook_delivery_ready: true".to_string()));
        assert!(lines.contains(&"hook_delivery_seen_for_cwd: true".to_string()));
        assert!(lines.contains(&"hook_delivery_event_count: 4".to_string()));
        assert!(lines.contains(&"hook_delivery_events_valid_json: true".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn backend_doctor_respects_hook_timeout_for_cortina_probe() {
        let script = unique_temp_path("slow-cortina.sh");
        fs::write(
            &script,
            "#!/bin/sh\nsleep 1\nprintf '%s' '{\"volva_hook_event_count\":1}'\n",
        )
        .expect("slow fake cortina script should be writable");

        let mut permissions = fs::metadata(&script)
            .expect("slow fake cortina metadata should be available")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("slow fake cortina should be executable");

        let mut config = VolvaConfig::default();
        config.backend.command = "/bin/echo".to_string();
        config.hook_adapter.enabled = true;
        config.hook_adapter.command = Some(script.to_string_lossy().to_string());
        config.hook_adapter.args = vec![
            "adapter".to_string(),
            "volva".to_string(),
            "hook-event".to_string(),
        ];
        config.hook_adapter.timeout_ms = 50;

        let lines = render_backend_doctor(
            &RuntimeBootstrap::new(config),
            Path::new("/tmp/volva-hook-health"),
        );

        assert!(lines.contains(&"local_backend_ready: true".to_string()));
        assert!(lines.contains(&"backend_ready: false".to_string()));
        assert!(lines.contains(&"hook_delivery_probe: cortina-probe-failed".to_string()));
        assert!(lines.contains(&"hook_delivery_ready: false".to_string()));
        assert!(lines.iter().any(
            |line| line.contains("hook_delivery_detail: status probe failed:")
                && line.contains("timed out")
        ));
    }

    #[test]
    fn command_resolved_accepts_real_binary_path() {
        assert!(command_resolved("/bin/echo"));
    }

    #[test]
    fn command_resolved_rejects_missing_binary_path() {
        assert!(!command_resolved("/definitely/not/a/real/binary"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_pathext_suffixes_normalize_extensions() {
        let suffixes = super::windows_pathext_suffixes();
        assert!(!suffixes.is_empty());
        assert!(suffixes.iter().all(|suffix| suffix.starts_with('.')));
    }

    #[cfg(unix)]
    #[test]
    fn command_resolved_rejects_non_executable_file() {
        let path = unique_temp_path("not-executable");
        fs::write(&path, "#!/bin/sh\necho no\n").expect("temp file should be writable");

        let mut permissions = fs::metadata(&path)
            .expect("temp file metadata should be available")
            .permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&path, permissions).expect("temp file permissions should update");

        assert!(!command_resolved(path.to_string_lossy().as_ref()));
    }

    #[cfg(unix)]
    fn unique_temp_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("volva-backend-{stamp}-{name}"))
    }
}
