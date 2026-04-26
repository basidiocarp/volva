use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use volva_core::{Checkpoint, CheckpointError, CheckpointSaver};

/// SQLite-backed checkpoint saver.
///
/// Stores checkpoints in a `SQLite` database, supporting both in-memory
/// (for testing) and file-based (for production) backends.
pub struct SqliteCheckpointSaver {
    conn: Mutex<Connection>,
}

impl SqliteCheckpointSaver {
    /// Create an in-memory checkpoint saver (for testing).
    pub fn new_in_memory() -> Result<Self, CheckpointError> {
        let conn = Connection::open_in_memory().map_err(|e| {
            CheckpointError::Storage(format!("failed to create in-memory database: {e}"))
        })?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open or create a checkpoint saver backed by a file.
    pub fn open(path: &Path) -> Result<Self, CheckpointError> {
        let conn = Connection::open(path).map_err(|e| {
            CheckpointError::Storage(format!(
                "failed to open database at {}: {e}",
                path.display()
            ))
        })?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> Result<(), CheckpointError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS checkpoints (
                checkpoint_id TEXT PRIMARY KEY,
                thread_id     TEXT NOT NULL,
                version       INTEGER NOT NULL,
                state         TEXT NOT NULL,
                metadata      TEXT NOT NULL,
                created_at    INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_checkpoints_thread
                ON checkpoints(thread_id, version DESC);",
        )
        .map_err(|e| CheckpointError::Storage(format!("failed to initialize schema: {e}")))?;
        Ok(())
    }

    fn checkpoint_from_row(row: &rusqlite::Row) -> Result<Checkpoint, rusqlite::Error> {
        let state_str: String = row.get(3)?;
        let metadata_str: String = row.get(4)?;
        let version_i64: i64 = row.get(2)?;
        #[allow(clippy::cast_sign_loss)]
        let version = if version_i64 >= 0 {
            version_i64 as u64
        } else {
            0
        };

        Ok(Checkpoint {
            checkpoint_id: row.get(0)?,
            thread_id: row.get(1)?,
            version,
            state: serde_json::from_str(&state_str).unwrap_or(serde_json::json!({})),
            metadata: serde_json::from_str(&metadata_str).unwrap_or_default(),
            created_at: row.get(5)?,
        })
    }
}

impl CheckpointSaver for SqliteCheckpointSaver {
    fn save(&self, checkpoint: &Checkpoint) -> Result<(), CheckpointError> {
        let conn = self.conn.lock().map_err(|e| {
            CheckpointError::Storage(format!("failed to acquire database lock: {e}"))
        })?;

        let state_json = serde_json::to_string(&checkpoint.state)
            .map_err(|e| CheckpointError::Storage(format!("failed to serialize state: {e}")))?;
        let metadata_json = serde_json::to_string(&checkpoint.metadata)
            .map_err(|e| CheckpointError::Storage(format!("failed to serialize metadata: {e}")))?;

        let version_i64 = i64::try_from(checkpoint.version).unwrap_or(i64::MAX);
        conn.execute(
            "INSERT OR REPLACE INTO checkpoints (checkpoint_id, thread_id, version, state, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                &checkpoint.checkpoint_id,
                &checkpoint.thread_id,
                version_i64,
                state_json,
                metadata_json,
                checkpoint.created_at,
            ],
        )
        .map_err(|e| CheckpointError::Storage(format!("failed to save checkpoint: {e}")))?;

        Ok(())
    }

    fn load(&self, thread_id: &str) -> Result<Option<Checkpoint>, CheckpointError> {
        let conn = self.conn.lock().map_err(|e| {
            CheckpointError::Storage(format!("failed to acquire database lock: {e}"))
        })?;

        let mut stmt = conn
            .prepare("SELECT checkpoint_id, thread_id, version, state, metadata, created_at FROM checkpoints WHERE thread_id = ?1 ORDER BY version DESC LIMIT 1")
            .map_err(|e| CheckpointError::Storage(format!("failed to prepare query: {e}")))?;

        let result = stmt
            .query_row([thread_id], Self::checkpoint_from_row)
            .optional()
            .map_err(|e| CheckpointError::Storage(format!("failed to load checkpoint: {e}")))?;

        Ok(result)
    }

    fn load_by_id(&self, checkpoint_id: &str) -> Result<Option<Checkpoint>, CheckpointError> {
        let conn = self.conn.lock().map_err(|e| {
            CheckpointError::Storage(format!("failed to acquire database lock: {e}"))
        })?;

        let mut stmt = conn
            .prepare("SELECT checkpoint_id, thread_id, version, state, metadata, created_at FROM checkpoints WHERE checkpoint_id = ?1")
            .map_err(|e| CheckpointError::Storage(format!("failed to prepare query: {e}")))?;

        let result = stmt
            .query_row([checkpoint_id], Self::checkpoint_from_row)
            .optional()
            .map_err(|e| CheckpointError::Storage(format!("failed to load checkpoint: {e}")))?;

        Ok(result)
    }

    fn list(&self, thread_id: &str) -> Result<Vec<Checkpoint>, CheckpointError> {
        let conn = self.conn.lock().map_err(|e| {
            CheckpointError::Storage(format!("failed to acquire database lock: {e}"))
        })?;

        let mut stmt = conn
            .prepare("SELECT checkpoint_id, thread_id, version, state, metadata, created_at FROM checkpoints WHERE thread_id = ?1 ORDER BY version DESC")
            .map_err(|e| CheckpointError::Storage(format!("failed to prepare query: {e}")))?;

        let checkpoints = stmt
            .query_map([thread_id], Self::checkpoint_from_row)
            .map_err(|e| CheckpointError::Storage(format!("failed to query checkpoints: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CheckpointError::Storage(format!("failed to collect checkpoints: {e}")))?;

        Ok(checkpoints)
    }

    fn delete_thread(&self, thread_id: &str) -> Result<(), CheckpointError> {
        let conn = self.conn.lock().map_err(|e| {
            CheckpointError::Storage(format!("failed to acquire database lock: {e}"))
        })?;

        conn.execute("DELETE FROM checkpoints WHERE thread_id = ?1", [thread_id])
            .map_err(|e| CheckpointError::Storage(format!("failed to delete checkpoints: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_checkpoint(checkpoint_id: &str, thread_id: &str, version: u64) -> Checkpoint {
        Checkpoint {
            checkpoint_id: checkpoint_id.to_string(),
            thread_id: thread_id.to_string(),
            version,
            state: serde_json::json!({ "status": "running" }),
            metadata: {
                let mut m = HashMap::new();
                m.insert("key".to_string(), serde_json::json!("value"));
                m
            },
            created_at: 1_234_567_890,
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let saver = SqliteCheckpointSaver::new_in_memory().expect("should create in-memory saver");
        let checkpoint = test_checkpoint("cp-1", "thread-1", 1);

        saver.save(&checkpoint).expect("should save checkpoint");
        let loaded = saver
            .load("thread-1")
            .expect("should load checkpoint")
            .expect("should have loaded checkpoint");

        assert_eq!(loaded.checkpoint_id, "cp-1");
        assert_eq!(loaded.thread_id, "thread-1");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.state, serde_json::json!({ "status": "running" }));
        assert_eq!(loaded.created_at, 1_234_567_890);
    }

    #[test]
    fn list_newest_first() {
        let saver = SqliteCheckpointSaver::new_in_memory().expect("should create in-memory saver");

        saver.save(&test_checkpoint("cp-1", "thread-1", 1)).ok();
        saver.save(&test_checkpoint("cp-2", "thread-1", 2)).ok();
        saver.save(&test_checkpoint("cp-3", "thread-1", 3)).ok();

        let list = saver.list("thread-1").expect("should list checkpoints");

        assert_eq!(list.len(), 3);
        assert_eq!(list[0].version, 3);
        assert_eq!(list[1].version, 2);
        assert_eq!(list[2].version, 1);
    }

    #[test]
    fn delete_thread_removes_all() {
        let saver = SqliteCheckpointSaver::new_in_memory().expect("should create in-memory saver");

        saver.save(&test_checkpoint("cp-1", "thread-1", 1)).ok();
        saver.save(&test_checkpoint("cp-2", "thread-1", 2)).ok();
        saver.save(&test_checkpoint("cp-3", "thread-2", 1)).ok();

        saver
            .delete_thread("thread-1")
            .expect("should delete thread");

        let thread1_list = saver.list("thread-1").expect("should list");
        let thread2_list = saver.list("thread-2").expect("should list");

        assert!(thread1_list.is_empty());
        assert_eq!(thread2_list.len(), 1);
    }

    #[test]
    fn load_by_id_finds_checkpoint() {
        let saver = SqliteCheckpointSaver::new_in_memory().expect("should create in-memory saver");
        let checkpoint = test_checkpoint("cp-1", "thread-1", 1);

        saver.save(&checkpoint).expect("should save checkpoint");
        let loaded = saver
            .load_by_id("cp-1")
            .expect("should load checkpoint")
            .expect("should have loaded checkpoint");

        assert_eq!(loaded.checkpoint_id, "cp-1");
    }

    #[test]
    fn load_returns_none_for_missing_thread() {
        let saver = SqliteCheckpointSaver::new_in_memory().expect("should create in-memory saver");

        let loaded = saver.load("missing-thread").expect("should load");
        assert!(loaded.is_none());
    }
}
