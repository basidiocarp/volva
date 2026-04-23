use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const OAUTH_BETA_HEADER_NAME: &str = "anthropic-beta";
pub const OAUTH_BETA_HEADER_VALUE: &str = "oauth-2025-04-20";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum OperationMode {
    #[default]
    Baseline,
    Orchestration,
}

impl fmt::Display for OperationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Baseline => f.write_str("baseline"),
            Self::Orchestration => f.write_str("orchestration"),
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSessionId(pub String);

impl ExecutionSessionId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn generate(mode: ExecutionMode) -> Self {
        Self(format!("volva-{mode}-{}", Uuid::new_v4()))
    }
}

impl fmt::Display for ExecutionSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBinding {
    pub workspace_root: String,
    pub worktree_id: Option<String>,
}

impl WorkspaceBinding {
    #[must_use]
    pub fn from_root(root: impl AsRef<Path>) -> Self {
        Self {
            workspace_root: root.as_ref().display().to_string(),
            worktree_id: None,
        }
    }

    #[must_use]
    pub fn with_worktree_id(mut self, worktree_id: Option<String>) -> Self {
        self.worktree_id = worktree_id.filter(|value| !value.trim().is_empty());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionParticipantIdentity {
    pub participant_id: String,
    pub host_kind: String,
}

impl fmt::Display for ExecutionParticipantIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.participant_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Run,
    Chat,
    BackendStatus,
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run => f.write_str("run"),
            Self::Chat => f.write_str("chat"),
            Self::BackendStatus => f.write_str("backend-status"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSessionState {
    Planned,
    Active,
    Paused,
    Resumed,
    Finished,
}

impl fmt::Display for ExecutionSessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planned => f.write_str("planned"),
            Self::Active => f.write_str("active"),
            Self::Paused => f.write_str("paused"),
            Self::Resumed => f.write_str("resumed"),
            Self::Finished => f.write_str("finished"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSessionIdentity {
    pub session_id: ExecutionSessionId,
    pub mode: ExecutionMode,
    pub backend: BackendKind,
    pub workspace: WorkspaceBinding,
    pub primary_participant: ExecutionParticipantIdentity,
    pub state: ExecutionSessionState,
}

impl ExecutionSessionIdentity {
    #[must_use]
    pub fn new(
        mode: ExecutionMode,
        backend: BackendKind,
        workspace: WorkspaceBinding,
        primary_participant: ExecutionParticipantIdentity,
        state: ExecutionSessionState,
    ) -> Self {
        Self {
            session_id: ExecutionSessionId::generate(mode),
            mode,
            backend,
            workspace,
            primary_participant,
            state,
        }
    }

    #[must_use]
    pub fn with_state(mut self, state: ExecutionSessionState) -> Self {
        self.state = state;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionId,
        ExecutionSessionIdentity, ExecutionSessionState, WorkspaceBinding,
    };

    #[test]
    fn execution_session_identity_captures_workspace_and_state() {
        let session = ExecutionSessionIdentity::new(
            ExecutionMode::Run,
            BackendKind::OfficialCli,
            WorkspaceBinding::from_root("/tmp/project").with_worktree_id(Some("wt-1".to_string())),
            ExecutionParticipantIdentity {
                participant_id: "operator@volva".to_string(),
                host_kind: "volva".to_string(),
            },
            ExecutionSessionState::Active,
        );

        assert!(session.session_id.0.starts_with("volva-run-"));
        assert_eq!(session.workspace.workspace_root, "/tmp/project");
        assert_eq!(session.workspace.worktree_id.as_deref(), Some("wt-1"));
        assert_eq!(session.primary_participant.participant_id, "operator@volva");
        assert_eq!(session.state, ExecutionSessionState::Active);
    }

    #[test]
    fn execution_session_id_generation_is_not_timestamp_shaped() {
        let first = ExecutionSessionId::generate(ExecutionMode::Run);
        let second = ExecutionSessionId::generate(ExecutionMode::Run);
        let suffix: &str = first.0.strip_prefix("volva-run-").unwrap_or_default();

        assert_ne!(first, second);
        assert!(first.0.starts_with("volva-run-"));
        assert!(first.0.len() > "volva-run-0000000000000".len());
        assert!(!suffix.chars().all(|ch: char| ch.is_ascii_digit()));
    }
}
