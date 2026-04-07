use std::fmt;

use serde::{Deserialize, Serialize};

pub const OAUTH_BETA_HEADER_NAME: &str = "anthropic-beta";
pub const OAUTH_BETA_HEADER_VALUE: &str = "oauth-2025-04-20";

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    OfficialCli,
    AnthropicApi,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OfficialCli => f.write_str("official-cli"),
            Self::AnthropicApi => f.write_str("anthropic-api"),
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthProvider {
    Anthropic,
}

impl fmt::Display for AuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Anthropic => f.write_str("anthropic"),
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthTarget {
    ClaudeAi,
    Console,
}

impl fmt::Display for AuthTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClaudeAi => f.write_str("claude.ai"),
            Self::Console => f.write_str("console"),
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    ApiKey,
    BearerToken,
}

impl fmt::Display for AuthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey => f.write_str("api-key"),
            Self::BearerToken => f.write_str("bearer-token"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCredentialSource {
    EnvironmentApiKey,
    StoredCredential,
}

impl fmt::Display for AuthCredentialSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvironmentApiKey => f.write_str("environment-api-key"),
            Self::StoredCredential => f.write_str("saved-credential"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredentialMetadata {
    pub provider: AuthProvider,
    pub target: AuthTarget,
    pub auth_mode: Option<AuthMode>,
    pub email: Option<String>,
    pub organization_id: Option<String>,
    pub subscription_type: Option<String>,
    pub expires_at: Option<u64>,
    pub expired: bool,
    pub has_refresh_token: bool,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStatus {
    pub provider: AuthProvider,
    pub logged_in: bool,
    pub active_credential_source: Option<AuthCredentialSource>,
    pub active_auth_mode: Option<AuthMode>,
    pub saved_credential: Option<StoredCredentialMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedCredential {
    pub mode: AuthMode,
    pub secret: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub auth_ready: bool,
    pub builtin_tool_count: usize,
    pub adapter_count: usize,
    pub bridge_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusLine {
    pub label: String,
    pub value: String,
}

impl StatusLine {
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}
