mod auth;
mod backend;
mod chat;
mod run;
mod session;

use std::env;

use anyhow::Result;
use clap::{Parser, Subcommand};
use opentelemetry::global;
use spore::logging::{LogOutput, LoggingConfig, SpanContext, SpanEvents, root_span, workflow_span};
use tracing::Level;
use tracing_subscriber::{filter::LevelFilter, prelude::*};
use volva_compat::import_candidates;
use volva_config::{GlobalVolvaConfig, VolvaConfig};
use volva_runtime::RuntimeBootstrap;

use crate::auth::{AuthCommand, handle_auth};
use crate::backend::{BackendCommand, handle_backend, render_backend_doctor};
use crate::chat::{ChatCommand, handle_chat};
use crate::run::{RunCommand, handle_run};
use volva_core::OperationMode;

#[derive(Debug, Parser)]
#[command(
    name = "volva",
    version,
    about = "Claude-first runtime shell for Basidiocarp"
)]
struct Cli {
    #[arg(long, value_enum)]
    pub mode: Option<OperationMode>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Auth(AuthCommand),
    Backend(BackendCommand),
    Chat(ChatCommand),
    Run(RunCommand),
    Doctor,
    Paths,
}

/// Initialize logging with both fmt and OpenTelemetry layers.
/// This ensures tracing spans are bridged into the OpenTelemetry context.
fn init_logging_with_otel(config: &LoggingConfig) -> Result<()> {
    use std::io;
    use tracing_subscriber::fmt;

    let filter = resolve_logging_filter(config);
    let fmt_layer = fmt::layer()
        .compact()
        .with_writer(io::stderr)
        .with_span_events(map_span_events(config.span_events))
        .with_target(config.include_target);

    let tracer = global::tracer("volva");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer);

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| anyhow::anyhow!("failed to initialize tracing subscriber: {e}"))?;

    Ok(())
}

fn resolve_logging_filter(config: &LoggingConfig) -> LevelFilter {
    let env_var_name = config.env_var.clone().or_else(|| {
        config.app_name.as_deref().map(|name| {
            let normalized = name
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_uppercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            format!("{normalized}_LOG")
        })
    });

    let level_str = env_var_name
        .and_then(|name| std::env::var(&name).ok())
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| config.default_level.to_string().to_ascii_lowercase());

    match level_str.as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "info" => LevelFilter::INFO,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        _ => config
            .default_level
            .as_str()
            .parse()
            .unwrap_or(LevelFilter::WARN),
    }
}

fn map_span_events(events: SpanEvents) -> tracing_subscriber::fmt::format::FmtSpan {
    use tracing_subscriber::fmt::format::FmtSpan;
    match events {
        SpanEvents::Off => FmtSpan::NONE,
        SpanEvents::Lifecycle => FmtSpan::NEW | FmtSpan::CLOSE,
        SpanEvents::Full => FmtSpan::FULL,
    }
}

fn main() -> Result<()> {
    // Initialize OpenTelemetry first so we can add its layer to the subscriber.
    // NOTE: TelemetryInit has no Drop/flush. Span data survives because spore currently uses
    // SimpleSpanExporter (synchronous, per-span). If the exporter is changed to a batch pipeline,
    // add provider.force_flush() + provider.shutdown() before process exit.
    let telemetry = spore::telemetry::init_tracer("volva").unwrap_or_else(|e| {
        tracing::warn!("OTel init skipped: {}", e);
        spore::telemetry::TelemetryInit::disabled("volva")
    });

    // Initialize logging with OpenTelemetry layer if telemetry is enabled.
    let logging_config = LoggingConfig::for_app("volva", Level::WARN)
        .with_output(LogOutput::Stderr)
        .with_span_events(SpanEvents::Lifecycle);

    if telemetry.enabled {
        init_logging_with_otel(&logging_config)?;
    } else {
        spore::logging::init_with_config(logging_config);
    }
    let span_context = current_span_context();
    let _root_span = root_span(&span_context).entered();
    let cli = Cli::parse();
    let _workflow_span = workflow_span(command_name(cli.command.as_ref()), &span_context).entered();

    // Resolve mode from CLI flag, global config, or baseline default
    let global_config = GlobalVolvaConfig::load();
    let mode = cli.mode
        .or_else(|| global_config.operation_mode())
        .unwrap_or(OperationMode::Baseline);

    // Print mode announcement
    match mode {
        OperationMode::Baseline => {
            eprintln!("volva: baseline mode — mycelium, hyphae, and rhizome active");
        }
        OperationMode::Orchestration => {
            eprintln!("volva: orchestration mode — canopy connected, full memory budget active");
        }
    }

    match cli.command.unwrap_or(Command::Doctor) {
        Command::Auth(auth) => handle_auth(auth),
        Command::Backend(backend) => handle_backend(backend),
        Command::Chat(chat) => handle_chat(chat, mode),
        Command::Run(run) => handle_run(run, mode),
        Command::Doctor => {
            let root = env::current_dir()?;
            let config = VolvaConfig::load_from(&root)?;
            let runtime = RuntimeBootstrap::new(config);
            print_doctor(&runtime, &root);
            Ok(())
        }
        Command::Paths => {
            let root = env::current_dir()?;
            let config = VolvaConfig::load_from(&root)?;
            print_paths(&root, &config);
            Ok(())
        }
    }
}

fn current_span_context() -> SpanContext {
    let context = SpanContext::for_app("volva");
    match env::current_dir() {
        Ok(path) => context.with_workspace_root(path.display().to_string()),
        Err(_) => context,
    }
}

fn command_name(command: Option<&Command>) -> &'static str {
    match command {
        Some(Command::Auth(_)) => "auth",
        Some(Command::Backend(_)) => "backend",
        Some(Command::Chat(_)) => "chat",
        Some(Command::Run(_)) => "run",
        Some(Command::Doctor) | None => "doctor",
        Some(Command::Paths) => "paths",
    }
}

fn print_doctor(runtime: &RuntimeBootstrap, cwd: &std::path::Path) {
    for line in render_backend_doctor(runtime, cwd) {
        println!("{line}");
    }
}

fn print_paths(root: &std::path::Path, config: &VolvaConfig) {
    println!("workspace_root: {}", root.display());
    println!("vendor_dir: {}", config.vendor_dir.display());
    for candidate in import_candidates() {
        println!("claude_import_candidate: {}", candidate.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{
        AuthProviderArg, AuthSubcommand, LoginCommand, LogoutCommand, StatusCommand,
    };
    use crate::backend::{
        BackendArg, BackendSubcommand, DoctorSubcommand, SessionSubcommand, StatusSubcommand,
    };
    use crate::chat::ChatCommand;
    use crate::run::RunCommand;

    #[test]
    fn auth_login_parses_provider_explicit_surface() {
        let cli = Cli::try_parse_from(["volva", "auth", "login", "anthropic"])
            .expect("provider-explicit login should parse");

        match cli.command {
            Some(Command::Auth(AuthCommand {
                command:
                    AuthSubcommand::Login(LoginCommand {
                        provider: AuthProviderArg::Anthropic,
                        console: false,
                        no_browser: false,
                    }),
            })) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn auth_logout_parses_provider_explicit_surface() {
        let cli = Cli::try_parse_from(["volva", "auth", "logout", "anthropic"])
            .expect("provider-explicit logout should parse");

        match cli.command {
            Some(Command::Auth(AuthCommand {
                command:
                    AuthSubcommand::Logout(LogoutCommand {
                        provider: AuthProviderArg::Anthropic,
                    }),
            })) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn auth_status_parses_without_provider() {
        let cli =
            Cli::try_parse_from(["volva", "auth", "status"]).expect("status should parse cleanly");

        match cli.command {
            Some(Command::Auth(AuthCommand {
                command: AuthSubcommand::Status(StatusCommand { json: false }),
            })) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn chat_parses_prompt_words() {
        let cli = Cli::try_parse_from(["volva", "chat", "say", "hello"])
            .expect("chat prompt should parse");

        match cli.command {
            Some(Command::Chat(ChatCommand { prompt })) => {
                assert_eq!(prompt, vec!["say".to_string(), "hello".to_string()]);
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn backend_status_parses_cleanly() {
        let cli = Cli::try_parse_from(["volva", "backend", "status"])
            .expect("backend status should parse");

        match cli.command {
            Some(Command::Backend(BackendCommand {
                command: BackendSubcommand::Status(StatusSubcommand {}),
            })) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn backend_doctor_parses_cleanly() {
        let cli = Cli::try_parse_from(["volva", "backend", "doctor"])
            .expect("backend doctor should parse");

        match cli.command {
            Some(Command::Backend(BackendCommand {
                command: BackendSubcommand::Doctor(DoctorSubcommand {}),
            })) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn backend_session_parses_cleanly() {
        let cli = Cli::try_parse_from(["volva", "backend", "session"])
            .expect("backend session should parse");

        match cli.command {
            Some(Command::Backend(BackendCommand {
                command: BackendSubcommand::Session(SessionSubcommand { json: false }),
            })) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn backend_session_json_parses_cleanly() {
        let cli = Cli::try_parse_from(["volva", "backend", "session", "--json"])
            .expect("backend session json should parse");

        match cli.command {
            Some(Command::Backend(BackendCommand {
                command: BackendSubcommand::Session(SessionSubcommand { json: true }),
            })) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn run_parses_backend_override_and_prompt() {
        let cli = Cli::try_parse_from([
            "volva",
            "run",
            "--backend",
            "official-cli",
            "summarize",
            "the",
            "repo",
        ])
        .expect("run command should parse");

        match cli.command {
            Some(Command::Run(RunCommand { backend, prompt })) => {
                assert_eq!(backend, Some(BackendArg::OfficialCli));
                assert_eq!(
                    prompt,
                    vec![
                        "summarize".to_string(),
                        "the".to_string(),
                        "repo".to_string()
                    ]
                );
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }
}
