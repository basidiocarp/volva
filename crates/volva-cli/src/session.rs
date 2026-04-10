use std::env;
use std::path::Path;

use volva_core::{
    BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionIdentity,
    ExecutionSessionState, WorkspaceBinding,
};

pub(crate) fn session_for_workspace(
    cwd: &Path,
    mode: ExecutionMode,
    backend: BackendKind,
    state: ExecutionSessionState,
) -> ExecutionSessionIdentity {
    let workspace = WorkspaceBinding::from_root(cwd).with_worktree_id(
        env::var("VOLVA_WORKTREE_ID")
            .ok()
            .filter(|value| !value.trim().is_empty()),
    );
    let participant_id = env::var("VOLVA_PARTICIPANT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(default_participant_id);

    ExecutionSessionIdentity::new(
        mode,
        backend,
        workspace,
        ExecutionParticipantIdentity {
            participant_id,
            host_kind: "volva".to_string(),
        },
        state,
    )
}

fn default_participant_id() -> String {
    let user = env::var("USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "operator".to_string());
    format!("{user}@volva")
}
