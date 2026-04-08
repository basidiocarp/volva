use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde::Deserialize;
use spore::logging::{SpanContext, request_span, subprocess_span};
use tracing::warn;
use url::Url;
use volva_core::{AuthProvider, AuthTarget, OAUTH_BETA_HEADER_NAME, OAUTH_BETA_HEADER_VALUE};

use super::account::{AnthropicAccountPayload, AnthropicOrganizationPayload};

pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const CLAUDE_AI_INFERENCE_SCOPE: &str = "user:inference";

const CONSOLE_AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";
const CLAUDE_AI_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const API_KEY_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/create_api_key";
const CLAUDEAI_SUCCESS_URL: &str = "https://platform.claude.com/oauth/code/success?app=claude-code";
const CONSOLE_SUCCESS_URL: &str = "https://platform.claude.com/buy_credits\
    ?returnUrl=/oauth/code/success%3Fapp%3Dclaude-code";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_AI_SCOPES: &[&str] = &[
    "org:create_api_key",
    "user:profile",
    CLAUDE_AI_INFERENCE_SCOPE,
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
];
const CONSOLE_SCOPES: &[&str] = &["org:create_api_key", "user:profile"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationUrls {
    pub authorize: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenExchangeResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub account: Option<AnthropicAccountPayload>,
    #[serde(default)]
    pub organization: Option<AnthropicOrganizationPayload>,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateApiKeyResponse {
    raw_key: Option<String>,
}

#[must_use]
pub fn authorization_urls(
    target: AuthTarget,
    code_challenge: &str,
    state: &str,
    callback_url: &str,
) -> AuthorizationUrls {
    AuthorizationUrls {
        authorize: build_authorize_url(target, code_challenge, state, callback_url),
    }
}

#[must_use]
pub fn provider_storage_path() -> PathBuf {
    crate::storage::provider_tokens_path(AuthProvider::Anthropic)
}

#[must_use]
pub fn success_redirect_url(target: AuthTarget) -> &'static str {
    anthropic_target_config(target).success_url
}

#[must_use]
pub fn normalize_scopes(scope_header: Option<&str>) -> Vec<String> {
    scope_header
        .unwrap_or_default()
        .split_whitespace()
        .filter(|scope| !scope.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[must_use]
pub fn uses_bearer_scope(scopes: &[String]) -> bool {
    scopes
        .iter()
        .any(|scope| scope == CLAUDE_AI_INFERENCE_SCOPE)
}

#[must_use]
pub fn requested_scopes(target: AuthTarget) -> &'static [&'static str] {
    anthropic_target_config(target).scopes
}

pub async fn exchange_code(
    authorization_code: &str,
    state: &str,
    code_verifier: &str,
    redirect_uri: &str,
    span_context: &SpanContext,
) -> Result<TokenExchangeResponse> {
    let _request_span = request_span("anthropic_token_exchange", span_context).entered();
    let client = oauth_client()?;
    let response = client
        .post(TOKEN_URL)
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": authorization_code,
            "redirect_uri": redirect_uri,
            "client_id": CLIENT_ID,
            "code_verifier": code_verifier,
            "state": state,
        }))
        .send()
        .await
        .context("failed to exchange Anthropic authorization code")?;

    parse_json_response(response, "Anthropic token exchange").await
}

pub async fn create_api_key(access_token: &str, span_context: &SpanContext) -> Result<String> {
    let _request_span = request_span("anthropic_api_key_mint", span_context).entered();
    let client = oauth_client()?;
    let response = client
        .post(API_KEY_URL)
        .header("authorization", format!("Bearer {access_token}"))
        .header(OAUTH_BETA_HEADER_NAME, OAUTH_BETA_HEADER_VALUE)
        .send()
        .await
        .context("failed to request Anthropic API key")?;

    let payload: CreateApiKeyResponse =
        parse_json_response(response, "Anthropic API key mint").await?;
    payload
        .raw_key
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Anthropic API key mint returned no API key"))
}

pub fn try_open_browser(url: &str, span_context: &SpanContext) -> bool {
    #[cfg(target_os = "windows")]
    {
        let _subprocess_span = subprocess_span("powershell", span_context).entered();
        let command = format!("Start-Process '{}'", url.replace('\'', "''"));
        let launched = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &command])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok();
        if !launched {
            warn!("failed to launch browser for Anthropic OAuth");
        }
        return launched;
    }

    #[cfg(target_os = "macos")]
    {
        let _subprocess_span = subprocess_span("open", span_context).entered();
        let launched = std::process::Command::new("open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok();
        if !launched {
            warn!("failed to launch browser for Anthropic OAuth");
        }
        return launched;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _subprocess_span = subprocess_span("xdg-open", span_context).entered();
        let launched = std::process::Command::new("xdg-open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok();
        if !launched {
            warn!("failed to launch browser for Anthropic OAuth");
        }
        return launched;
    }

    #[cfg(not(any(target_os = "windows", unix)))]
    {
        let _ = url;
        let _ = span_context;
        false
    }
}

fn build_authorize_url(
    target: AuthTarget,
    code_challenge: &str,
    state: &str,
    redirect_uri: &str,
) -> String {
    let config = anthropic_target_config(target);

    let mut url =
        Url::parse(config.authorize_url).expect("Anthropic authorize URL constant should be valid");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("code", "true");
        query.append_pair("client_id", CLIENT_ID);
        query.append_pair("response_type", "code");
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("scope", &config.scopes.join(" "));
        query.append_pair("code_challenge", code_challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", state);
    }

    url.to_string()
}

struct AnthropicTargetConfig {
    authorize_url: &'static str,
    success_url: &'static str,
    scopes: &'static [&'static str],
}

fn anthropic_target_config(target: AuthTarget) -> AnthropicTargetConfig {
    match target {
        AuthTarget::ClaudeAi => AnthropicTargetConfig {
            authorize_url: CLAUDE_AI_AUTHORIZE_URL,
            success_url: CLAUDEAI_SUCCESS_URL,
            scopes: CLAUDE_AI_SCOPES,
        },
        AuthTarget::Console => AnthropicTargetConfig {
            authorize_url: CONSOLE_AUTHORIZE_URL,
            success_url: CONSOLE_SUCCESS_URL,
            scopes: CONSOLE_SCOPES,
        },
        _ => unreachable!("unsupported Anthropic auth target: {target}"),
    }
}

fn oauth_client() -> Result<Client> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("failed to build Anthropic OAuth HTTP client")
}

async fn parse_json_response<T>(response: reqwest::Response, context: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("{context} failed with {status}: {body}");
    }

    response
        .json::<T>()
        .await
        .with_context(|| format!("failed to parse {context} response"))
}

#[cfg(test)]
mod tests {
    use super::{
        CLAUDE_AI_INFERENCE_SCOPE, OAUTH_BETA_HEADER_VALUE, authorization_urls, normalize_scopes,
        provider_storage_path, requested_scopes, success_redirect_url, uses_bearer_scope,
    };
    use volva_core::AuthTarget;

    #[test]
    fn authorization_url_contains_expected_parameters() {
        let urls = authorization_urls(
            AuthTarget::ClaudeAi,
            "challenge123",
            "state456",
            "http://localhost:7777/callback",
        );
        let parsed = url::Url::parse(&urls.authorize).expect("authorize URL should parse");
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        let expected_scope = requested_scopes(AuthTarget::ClaudeAi).join(" ");

        assert!(urls.authorize.contains("client_id="));
        assert!(urls.authorize.contains("response_type=code"));
        assert!(urls.authorize.contains("code_challenge_method=S256"));
        assert!(urls.authorize.contains("state=state456"));
        assert!(
            urls.authorize
                .contains("redirect_uri=http%3A%2F%2Flocalhost%3A7777%2Fcallback")
        );
        assert_eq!(
            query.get("scope").map(String::as_str),
            Some(expected_scope.as_str())
        );
    }

    #[test]
    fn console_authorization_url_requests_console_scopes_only() {
        let urls = authorization_urls(
            AuthTarget::Console,
            "challenge123",
            "state456",
            "http://localhost:7777/callback",
        );
        let parsed = url::Url::parse(&urls.authorize).expect("authorize URL should parse");
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert_eq!(
            query.get("scope").map(String::as_str),
            Some("org:create_api_key user:profile")
        );
    }

    #[test]
    fn scope_normalization_handles_empty_values() {
        assert!(normalize_scopes(None).is_empty());
        assert_eq!(
            normalize_scopes(Some("user:profile user:inference")),
            vec![
                "user:profile".to_string(),
                CLAUDE_AI_INFERENCE_SCOPE.to_string()
            ],
        );
    }

    #[test]
    fn bearer_mode_is_derived_from_scopes() {
        assert!(uses_bearer_scope(&[CLAUDE_AI_INFERENCE_SCOPE.to_string()]));
        assert!(!uses_bearer_scope(&["org:create_api_key".to_string()]));
    }

    #[test]
    fn provider_paths_and_redirects_match_first_slice_contract() {
        assert!(
            provider_storage_path()
                .to_string_lossy()
                .ends_with(".volva/auth/anthropic.json")
        );
        assert!(success_redirect_url(AuthTarget::ClaudeAi).contains("oauth/code/success"));
        assert!(success_redirect_url(AuthTarget::Console).contains("buy_credits"));
        assert_eq!(OAUTH_BETA_HEADER_VALUE, "oauth-2025-04-20");
    }
}
