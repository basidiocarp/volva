use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Checkpoint durability mode.
///
/// Controls how aggressively checkpoints are persisted:
/// - `Sync`: Synchronously write to storage before returning
/// - `Async`: Defer writes to background; faster but may lose recent checkpoints on crash
/// - `Exit`: Only save checkpoints on normal shutdown
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointDurability {
    Sync,
    #[default]
    Async,
    Exit,
}

impl std::fmt::Display for CheckpointDurability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sync => f.write_str("sync"),
            Self::Async => f.write_str("async"),
            Self::Exit => f.write_str("exit"),
        }
    }
}

/// A saved checkpoint for execution recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub thread_id: String,
    pub version: u64,
    pub state: serde_json::Value,
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: i64,
}

/// Error type for checkpoint operations.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("not found: {0}")]
    NotFound(String),
}

/// Trait for persisting and loading checkpoints.
///
/// Implementations must be thread-safe (`Send + Sync`) since checkpoints
/// may be saved and loaded from multiple threads.
pub trait CheckpointSaver: Send + Sync {
    /// Save a checkpoint to persistent storage.
    fn save(&self, checkpoint: &Checkpoint) -> Result<(), CheckpointError>;

    /// Load the latest checkpoint for a given thread.
    fn load(&self, thread_id: &str) -> Result<Option<Checkpoint>, CheckpointError>;

    /// Load a checkpoint by its ID.
    fn load_by_id(&self, checkpoint_id: &str) -> Result<Option<Checkpoint>, CheckpointError>;

    /// List all checkpoints for a given thread, newest first.
    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError>;

    /// Delete all checkpoints for a given thread.
    fn delete_thread(&self, thread_id: &str) -> Result<(), CheckpointError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_durability_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&CheckpointDurability::Sync).unwrap(),
            "\"sync\""
        );
        assert_eq!(
            serde_json::to_string(&CheckpointDurability::Async).unwrap(),
            "\"async\""
        );
        assert_eq!(
            serde_json::to_string(&CheckpointDurability::Exit).unwrap(),
            "\"exit\""
        );
    }

    #[test]
    fn checkpoint_durability_defaults_to_async() {
        // Default is derived via the #[default] attribute
        let default_mode = CheckpointDurability::default();
        assert_eq!(default_mode, CheckpointDurability::Async);
    }

    #[test]
    fn checkpoint_durability_display_is_lowercase() {
        assert_eq!(CheckpointDurability::Sync.to_string(), "sync");
        assert_eq!(CheckpointDurability::Async.to_string(), "async");
        assert_eq!(CheckpointDurability::Exit.to_string(), "exit");
    }
}
