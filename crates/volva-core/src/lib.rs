use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod checkpoint;

pub use checkpoint::{Checkpoint, CheckpointDurability, CheckpointError, CheckpointSaver};

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
    #[serde(default = "default_workspace_id")]
    pub workspace_id: String,
    pub worktree_id: Option<String>,
}

fn default_workspace_id() -> String {
    String::new()
}

impl WorkspaceBinding {
    #[must_use]
    pub fn from_root(root: impl AsRef<Path>) -> Self {
        let path = root.as_ref();
        let workspace_root = path.display().to_string();
        let workspace_id = std::fs::canonicalize(path)
            .ok()
            .and_then(|p| p.to_str().map(ToString::to_string))
            .unwrap_or_else(|| workspace_root.clone());

        Self {
            workspace_root,
            workspace_id,
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
        AuthCredentialSource, AuthMode, AuthProvider, AuthTarget, BackendKind, ExecutionMode,
        ExecutionParticipantIdentity, ExecutionSessionId, ExecutionSessionIdentity,
        ExecutionSessionState, OperationMode, StatusLine, WorkspaceBinding,
    };

    // --- Display impls ---

    #[test]
    fn operation_mode_display() {
        assert_eq!(OperationMode::Baseline.to_string(), "baseline");
        assert_eq!(OperationMode::Orchestration.to_string(), "orchestration");
    }

    #[test]
    fn backend_kind_display() {
        assert_eq!(BackendKind::OfficialCli.to_string(), "official-cli");
        assert_eq!(BackendKind::AnthropicApi.to_string(), "anthropic-api");
    }

    #[test]
    fn auth_provider_display() {
        assert_eq!(AuthProvider::Anthropic.to_string(), "anthropic");
    }

    #[test]
    fn auth_target_display() {
        assert_eq!(AuthTarget::ClaudeAi.to_string(), "claude.ai");
        assert_eq!(AuthTarget::Console.to_string(), "console");
    }

    #[test]
    fn auth_mode_display() {
        assert_eq!(AuthMode::ApiKey.to_string(), "api-key");
        assert_eq!(AuthMode::BearerToken.to_string(), "bearer-token");
    }

    #[test]
    fn auth_credential_source_display() {
        assert_eq!(
            AuthCredentialSource::EnvironmentApiKey.to_string(),
            "environment-api-key"
        );
        assert_eq!(
            AuthCredentialSource::StoredCredential.to_string(),
            "saved-credential"
        );
    }

    #[test]
    fn execution_mode_display() {
        assert_eq!(ExecutionMode::Run.to_string(), "run");
        assert_eq!(ExecutionMode::Chat.to_string(), "chat");
        assert_eq!(ExecutionMode::BackendStatus.to_string(), "backend-status");
    }

    #[test]
    fn execution_session_state_display() {
        assert_eq!(ExecutionSessionState::Planned.to_string(), "planned");
        assert_eq!(ExecutionSessionState::Active.to_string(), "active");
        assert_eq!(ExecutionSessionState::Paused.to_string(), "paused");
        assert_eq!(ExecutionSessionState::Resumed.to_string(), "resumed");
        assert_eq!(ExecutionSessionState::Finished.to_string(), "finished");
    }

    #[test]
    fn execution_participant_identity_display_is_participant_id() {
        let identity = ExecutionParticipantIdentity {
            participant_id: "agent-42".to_string(),
            host_kind: "volva".to_string(),
        };
        assert_eq!(identity.to_string(), "agent-42");
    }

    // --- Default derivations ---

    #[test]
    fn operation_mode_defaults_to_baseline() {
        assert_eq!(OperationMode::default(), OperationMode::Baseline);
    }

    // --- Serde ---

    #[test]
    fn backend_kind_serializes_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&BackendKind::OfficialCli).unwrap(),
            "\"official-cli\""
        );
        assert_eq!(
            serde_json::to_string(&BackendKind::AnthropicApi).unwrap(),
            "\"anthropic-api\""
        );
    }

    #[test]
    fn backend_kind_roundtrips_via_serde() {
        let original = BackendKind::OfficialCli;
        let json = serde_json::to_string(&original).unwrap();
        let restored: BackendKind = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    // --- StatusLine ---

    #[test]
    fn status_line_new_sets_label_and_value() {
        let line = StatusLine::new("Backend", "official-cli");
        assert_eq!(line.label, "Backend");
        assert_eq!(line.value, "official-cli");
    }

    #[test]
    fn status_line_new_accepts_owned_strings() {
        let label = "Auth".to_string();
        let value = "api-key".to_string();
        let line = StatusLine::new(label, value);
        assert_eq!(line.label, "Auth");
        assert_eq!(line.value, "api-key");
    }

    // --- WorkspaceBinding ---

    #[test]
    fn workspace_binding_from_root_sets_workspace_root() {
        let binding = WorkspaceBinding::from_root("/tmp/myproject");
        assert_eq!(binding.workspace_root, "/tmp/myproject");
        assert!(!binding.workspace_id.is_empty());
        assert!(binding.worktree_id.is_none());
    }

    #[test]
    fn workspace_binding_with_worktree_id_some() {
        let binding =
            WorkspaceBinding::from_root("/tmp/p").with_worktree_id(Some("wt-1".to_string()));
        assert_eq!(binding.worktree_id.as_deref(), Some("wt-1"));
    }

    #[test]
    fn workspace_binding_with_worktree_id_none() {
        let binding = WorkspaceBinding::from_root("/tmp/p").with_worktree_id(None);
        assert!(binding.worktree_id.is_none());
    }

    #[test]
    fn workspace_binding_with_worktree_id_empty_string_filtered() {
        let binding =
            WorkspaceBinding::from_root("/tmp/p").with_worktree_id(Some(String::new()));
        assert!(binding.worktree_id.is_none());
    }

    #[test]
    fn workspace_binding_with_worktree_id_whitespace_only_filtered() {
        let binding =
            WorkspaceBinding::from_root("/tmp/p").with_worktree_id(Some("   ".to_string()));
        assert!(binding.worktree_id.is_none());
    }

    // --- ExecutionSessionIdentity (existing, kept) ---

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
        assert!(!session.workspace.workspace_id.is_empty());
        assert_eq!(session.workspace.worktree_id.as_deref(), Some("wt-1"));
        assert_eq!(session.primary_participant.participant_id, "operator@volva");
        assert_eq!(session.state, ExecutionSessionState::Active);
    }

    #[test]
    fn execution_session_identity_with_state_transitions() {
        let session = ExecutionSessionIdentity::new(
            ExecutionMode::Chat,
            BackendKind::AnthropicApi,
            WorkspaceBinding::from_root("/tmp/p"),
            ExecutionParticipantIdentity {
                participant_id: "agent".to_string(),
                host_kind: "volva".to_string(),
            },
            ExecutionSessionState::Planned,
        )
        .with_state(ExecutionSessionState::Finished);

        assert_eq!(session.state, ExecutionSessionState::Finished);
        assert!(session.session_id.0.starts_with("volva-chat-"));
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

    #[test]
    fn execution_session_id_as_str_matches_inner() {
        let id = ExecutionSessionId::generate(ExecutionMode::Run);
        assert_eq!(id.as_str(), id.0.as_str());
    }
}
