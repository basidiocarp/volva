use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use spore::logging::{SpanContext, workflow_span};
use tokio::runtime::Runtime;
use uuid::Uuid;
use volva_auth::{
    AnthropicLoginRequest, AnthropicLoginSession, auth_status, clear_tokens, save_tokens,
};
use volva_core::{AuthProvider, AuthStatus, AuthTarget};

#[derive(Debug, Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub command: AuthSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum AuthSubcommand {
    Login(LoginCommand),
    Logout(LogoutCommand),
    Status(StatusCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuthProviderArg {
    Anthropic,
}

impl From<AuthProviderArg> for AuthProvider {
    fn from(value: AuthProviderArg) -> Self {
        match value {
            AuthProviderArg::Anthropic => Self::Anthropic,
        }
    }
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct LoginCommand {
    #[arg(value_enum)]
    pub provider: AuthProviderArg,
    #[arg(long)]
    pub console: bool,
    #[arg(long)]
    pub no_browser: bool,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct LogoutCommand {
    #[arg(value_enum)]
    pub provider: AuthProviderArg,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct StatusCommand {
    #[arg(long)]
    pub json: bool,
}

pub fn handle_auth(auth: AuthCommand) -> Result<()> {
    let correlation_id = Uuid::new_v4().to_string();
    let span_context = auth_span_context().with_session_id(correlation_id.clone());
    let _workflow_span = workflow_span("handle_auth", &span_context).entered();
    match auth.command {
        AuthSubcommand::Login(command) => handle_login(command, &span_context, correlation_id),
        AuthSubcommand::Logout(command) => handle_logout(command, &span_context),
        AuthSubcommand::Status(command) => handle_status(command, &span_context),
    }
}

fn handle_login(
    command: LoginCommand,
    span_context: &SpanContext,
    correlation_id: String,
) -> Result<()> {
    let provider = AuthProvider::from(command.provider);
    match provider {
        AuthProvider::Anthropic => handle_anthropic_login(command, span_context, correlation_id),
        _ => unreachable!("unsupported auth provider"),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn handle_logout(command: LogoutCommand, span_context: &SpanContext) -> Result<()> {
    let _workflow_span = workflow_span("auth_logout", span_context).entered();
    let provider = AuthProvider::from(command.provider);
    clear_tokens(provider)?;
    println!("Cleared saved {provider} auth state.");
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn handle_status(command: StatusCommand, span_context: &SpanContext) -> Result<()> {
    let _workflow_span = workflow_span("auth_status", span_context).entered();
    let status = auth_status(AuthProvider::Anthropic)?;
    render_status(&status, command.json)
}

#[allow(clippy::needless_pass_by_value)]
fn handle_anthropic_login(
    command: LoginCommand,
    span_context: &SpanContext,
    correlation_id: String,
) -> Result<()> {
    let span_context = span_context.clone().with_tool("anthropic-auth");
    let _workflow_span = workflow_span("anthropic_login", &span_context).entered();
    let target = if command.console {
        AuthTarget::Console
    } else {
        AuthTarget::ClaudeAi
    };
    let runtime = Runtime::new()?;
    let _workflow_span = workflow_span("anthropic_login_start", &span_context).entered();
    let session = runtime.block_on(AnthropicLoginSession::start(AnthropicLoginRequest {
        target,
        open_browser: !command.no_browser,
        correlation_id: Some(correlation_id),
    }))?;

    if command.no_browser {
        println!("Browser launch disabled.");
    } else if session.browser_open_attempted() {
        println!("Attempted to open your browser for {target} authentication.");
    } else {
        println!("Automatic browser launch failed. Open the URL below manually.");
    }

    println!();
    println!("Authenticate by visiting:");
    println!("  {}", session.authorization_urls().authorize);
    println!();
    println!("Waiting for callback on:");
    println!("  {}", session.callback_url()?);

    let _workflow_span = workflow_span("anthropic_login_complete", &span_context).entered();
    let completion = runtime.block_on(session.complete())?;
    let saved_path = save_tokens(AuthProvider::Anthropic, &completion.tokens)?;

    println!();
    println!("Logged in to anthropic via {}.", completion.result.target);
    println!("Auth mode: {}", completion.result.credential_mode);
    println!("Saved credentials: {}", saved_path.display());
    if let Some(email) = &completion.result.account_email {
        println!("Email: {email}");
    }
    if let Some(organization_id) = &completion.result.organization_id {
        println!("Organization ID: {organization_id}");
    }
    if let Some(subscription_type) = &completion.result.subscription_type {
        println!("Subscription type: {subscription_type}");
    }

    Ok(())
}

fn auth_span_context() -> SpanContext {
    let context = SpanContext::for_app("volva").with_tool("auth");
    match std::env::current_dir() {
        Ok(path) => context.with_workspace_root(path.display().to_string()),
        Err(_) => context,
    }
}

fn render_status(status: &AuthStatus, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(status)?);
        return Ok(());
    }

    println!("Provider: {}", status.provider);
    println!(
        "Authenticated: {}",
        if status.logged_in { "yes" } else { "no" }
    );
    println!(
        "Active source: {}",
        status
            .active_credential_source
            .map_or_else(|| "none".to_string(), |source| source.to_string())
    );
    println!(
        "Active mode: {}",
        status
            .active_auth_mode
            .map_or_else(|| "none".to_string(), |mode| mode.to_string())
    );

    if let Some(saved) = &status.saved_credential {
        println!("Saved credentials: present");
        println!("Saved target: {}", saved.target);
        println!(
            "Saved auth mode: {}",
            saved
                .auth_mode
                .map_or_else(|| "none".to_string(), |mode| mode.to_string())
        );
        println!(
            "Saved expired: {}",
            if saved.expired { "yes" } else { "no" }
        );
        println!(
            "Saved refresh token: {}",
            if saved.has_refresh_token { "yes" } else { "no" }
        );
        println!(
            "Saved API key: {}",
            if saved.has_api_key { "yes" } else { "no" }
        );

        if let Some(email) = &saved.email {
            println!("Email: {email}");
        }
        if let Some(organization_id) = &saved.organization_id {
            println!("Organization ID: {organization_id}");
        }
        if let Some(subscription_type) = &saved.subscription_type {
            println!("Subscription type: {subscription_type}");
        }
        if let Some(expires_at) = saved.expires_at {
            println!("Saved expires at: {expires_at}");
        }
    } else {
        println!("Saved credentials: absent");
    }

    Ok(())
}
