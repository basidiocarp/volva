mod auth;
mod backend;
mod chat;
mod run;

use std::env;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use spore::logging::{LogOutput, LoggingConfig, SpanContext, SpanEvents, root_span, workflow_span};
use tracing::Level;
use volva_compat::import_candidates;
use volva_config::VolvaConfig;
use volva_runtime::RuntimeBootstrap;

use crate::auth::{AuthCommand, handle_auth};
use crate::backend::{BackendCommand, handle_backend, render_backend_doctor};
use crate::chat::{ChatCommand, handle_chat};
use crate::run::{RunCommand, handle_run};

#[derive(Debug, Parser)]
#[command(
    name = "volva",
    version,
    about = "Claude-first runtime shell for Basidiocarp"
)]
struct Cli {
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

fn main() -> Result<()> {
    spore::logging::init_with_config(
        LoggingConfig::for_app("volva", Level::WARN)
            .with_output(LogOutput::Stderr)
            .with_span_events(SpanEvents::Lifecycle),
    );
    let span_context = current_span_context();
    let _root_span = root_span(&span_context).entered();
    let cli = Cli::parse();
    let _workflow_span = workflow_span(command_name(cli.command.as_ref()), &span_context).entered();

    match cli.command.unwrap_or(Command::Doctor) {
        Command::Auth(auth) => handle_auth(auth),
        Command::Backend(backend) => handle_backend(backend),
        Command::Chat(chat) => handle_chat(chat),
        Command::Run(run) => handle_run(run),
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
            print_paths(root, &config);
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

fn print_paths(root: PathBuf, config: &VolvaConfig) {
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
    use crate::backend::{BackendArg, BackendSubcommand, DoctorSubcommand, StatusSubcommand};
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
