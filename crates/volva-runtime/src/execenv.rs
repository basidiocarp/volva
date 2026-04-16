//! Execution environment isolation for volva task sessions.
//!
//! An [`ExecEnv`] owns the lifecycle of an isolated working directory for a
//! task: directory tree setup, provider-native config injection, skill
//! injection, worktree management, and GC metadata.  It cleans up on
//! [`Drop`] unless [`ExecEnv::keep`] was called first.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// The agent provider that will run inside this execution environment.
///
/// Each variant determines which provider-native context files are written
/// into the working directory during [`ExecEnv::inject_provider_config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Provider {
    /// Anthropic Claude CLI — writes `CLAUDE.md`.
    Claude,
    /// `OpenAI` Codex CLI — writes `AGENTS.md`.
    Codex,
    /// Google Gemini CLI — writes `GEMINI.md`.
    Gemini,
}

impl Provider {
    /// The provider-native context file name for this provider.
    #[must_use]
    pub fn context_file_name(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE.md",
            Self::Codex => "AGENTS.md",
            Self::Gemini => "GEMINI.md",
        }
    }
}

// ---------------------------------------------------------------------------
// ExecEnvConfig
// ---------------------------------------------------------------------------

/// Configuration for creating an [`ExecEnv`].
#[derive(Debug, Clone)]
pub struct ExecEnvConfig {
    /// The provider that will run inside the environment.
    pub provider: Provider,
    /// Optional path to a provider-native context file on the host.  When
    /// `None` or when the path does not exist, provider config injection is
    /// skipped gracefully rather than failing.
    pub provider_config_source: Option<PathBuf>,
    /// Skill directories to inject, keyed by a logical name.  Each entry is a
    /// directory whose contents are copied into a provider-native skill
    /// subdirectory within the [`ExecEnv`] working directory.  Non-existent
    /// source directories are skipped gracefully.
    pub skill_sources: Vec<PathBuf>,
    /// The task identifier used in GC metadata and directory naming.
    pub task_id: String,
    /// The base directory under which the isolated working directory is
    /// created.  Defaults to the platform temp directory when not set.
    pub base_dir: Option<PathBuf>,
}

impl ExecEnvConfig {
    /// Construct a minimal config for the given provider and task id.
    #[must_use]
    pub fn new(provider: Provider, task_id: impl Into<String>) -> Self {
        Self {
            provider,
            provider_config_source: None,
            skill_sources: Vec::new(),
            task_id: task_id.into(),
            base_dir: None,
        }
    }

    /// Set the provider config source path.
    #[must_use]
    pub fn with_provider_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.provider_config_source = Some(path.into());
        self
    }

    /// Append a skill source directory.
    #[must_use]
    pub fn with_skill_source(mut self, path: impl Into<PathBuf>) -> Self {
        self.skill_sources.push(path.into());
        self
    }

    /// Override the base directory for environment creation.
    #[must_use]
    pub fn with_base_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(path.into());
        self
    }
}

// ---------------------------------------------------------------------------
// GcMetadata
// ---------------------------------------------------------------------------

/// Metadata written to `gc-metadata.json` inside the isolated directory.
///
/// A future sweeper (owned by hymenium) can read these records to identify
/// stale environments for deferred cleanup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GcMetadata {
    /// Seconds since the Unix epoch when the environment was created.
    pub created_at_secs: u64,
    /// The task id that owns this environment.
    pub task_id: String,
    /// The provider that was configured for this environment.
    pub provider: String,
}

// ---------------------------------------------------------------------------
// ExecEnv
// ---------------------------------------------------------------------------

/// An isolated execution environment for a single task session.
///
/// The environment owns a working directory on disk.  It is automatically
/// removed when dropped unless [`ExecEnv::keep`] was called to transfer
/// ownership out.
///
/// # Examples
///
/// ```no_run
/// use volva_runtime::execenv::{ExecEnv, ExecEnvConfig, Provider};
///
/// let config = ExecEnvConfig::new(Provider::Claude, "task-abc-123");
/// let env = ExecEnv::create(config).expect("env should be created");
/// println!("working dir: {}", env.working_dir().display());
/// // env is removed when dropped.
/// ```
#[derive(Debug)]
pub struct ExecEnv {
    working_dir: PathBuf,
    provider: Provider,
    task_id: String,
    /// When `true` the working directory is removed on drop.
    cleanup_on_drop: bool,
}

impl ExecEnv {
    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// Create a new isolated execution environment.
    ///
    /// Steps performed:
    /// 1. Creates the working directory.
    /// 2. Injects provider-native config (skips gracefully if source is
    ///    absent or not provided).
    /// 3. Injects skill files (skips gracefully per missing source).
    /// 4. Writes GC metadata.
    pub fn create(config: ExecEnvConfig) -> Result<Self> {
        let ExecEnvConfig {
            provider,
            provider_config_source,
            skill_sources,
            task_id,
            base_dir,
        } = config;

        let working_dir = build_working_dir_path(provider, &task_id, base_dir.as_deref());
        fs::create_dir_all(&working_dir).with_context(|| {
            format!(
                "failed to create exec env working directory `{}`",
                working_dir.display()
            )
        })?;

        let env = Self {
            working_dir,
            provider,
            task_id,
            cleanup_on_drop: true,
        };

        env.inject_provider_config(provider_config_source.as_deref())?;
        env.inject_skills(&skill_sources)?;
        env.write_gc_metadata()?;

        Ok(env)
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    /// The path to the isolated working directory.
    #[must_use]
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// The provider configured for this environment.
    #[must_use]
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// The task id associated with this environment.
    #[must_use]
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    // ------------------------------------------------------------------
    // Provider config injection
    // ------------------------------------------------------------------

    /// Write the provider-native context file into the working directory.
    ///
    /// If `source` is `None` or the source path does not exist, the step is
    /// skipped without error — missing provider content is not a hard failure.
    pub fn inject_provider_config(&self, source: Option<&Path>) -> Result<()> {
        let source = match source {
            Some(p) if p.exists() => p,
            _ => return Ok(()),
        };

        let dest = self.working_dir.join(self.provider.context_file_name());
        fs::copy(source, &dest).with_context(|| {
            format!(
                "failed to copy provider config from `{}` to `{}`",
                source.display(),
                dest.display()
            )
        })?;

        Ok(())
    }

    // ------------------------------------------------------------------
    // Skill injection
    // ------------------------------------------------------------------

    /// Inject skill files from all configured source directories.
    ///
    /// For each source directory that exists, every file at the top level is
    /// copied into a `skills/` subdirectory within the working directory.
    /// Non-existent source directories are skipped gracefully.
    pub fn inject_skills(&self, sources: &[PathBuf]) -> Result<()> {
        for source_dir in sources {
            self.inject_skill_dir(source_dir)?;
        }
        Ok(())
    }

    fn inject_skill_dir(&self, source_dir: &Path) -> Result<()> {
        if !source_dir.exists() {
            return Ok(());
        }

        let skills_dir = self.working_dir.join("skills");
        fs::create_dir_all(&skills_dir).with_context(|| {
            format!(
                "failed to create skills directory `{}`",
                skills_dir.display()
            )
        })?;

        let entries = fs::read_dir(source_dir).with_context(|| {
            format!(
                "failed to read skill source directory `{}`",
                source_dir.display()
            )
        })?;

        for entry in entries {
            let entry = entry.with_context(|| {
                format!(
                    "failed to read entry in skill source directory `{}`",
                    source_dir.display()
                )
            })?;
            let src = entry.path();
            if src.is_file() {
                let file_name = entry.file_name();
                let dest = skills_dir.join(&file_name);
                fs::copy(&src, &dest).with_context(|| {
                    format!(
                        "failed to copy skill file from `{}` to `{}`",
                        src.display(),
                        dest.display()
                    )
                })?;
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Worktree management
    // ------------------------------------------------------------------

    /// Set up (or reuse) a git worktree scoped to this task session.
    ///
    /// The worktree is created at `<working_dir>/worktree` and linked to
    /// `repo_root` at the given `branch`.  If the worktree directory already
    /// exists, the call succeeds without re-creating it.
    ///
    /// On failure (e.g. git is unavailable or the repo root is not a git
    /// repo) the error is returned so the caller can decide whether to
    /// treat it as fatal.
    pub fn setup_worktree(&self, repo_root: &Path, branch: &str) -> Result<PathBuf> {
        let worktree_path = self.working_dir.join("worktree");

        if worktree_path.exists() {
            return Ok(worktree_path);
        }

        let output = std::process::Command::new("git")
            .args([
                "-C",
                &repo_root.display().to_string(),
                "worktree",
                "add",
                &worktree_path.display().to_string(),
                branch,
            ])
            .output()
            .with_context(|| {
                format!(
                    "failed to launch git worktree add for `{}`",
                    worktree_path.display()
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "git worktree add failed for `{}`: {stderr}",
                worktree_path.display()
            );
        }

        Ok(worktree_path)
    }

    // ------------------------------------------------------------------
    // GC metadata
    // ------------------------------------------------------------------

    fn write_gc_metadata(&self) -> Result<()> {
        let created_at_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let metadata = GcMetadata {
            created_at_secs,
            task_id: self.task_id.clone(),
            provider: self.provider.context_file_name().to_string(),
        };

        let dest = self.working_dir.join("gc-metadata.json");
        let payload =
            serde_json::to_vec_pretty(&metadata).context("failed to serialize gc metadata")?;
        fs::write(&dest, payload)
            .with_context(|| format!("failed to write gc metadata to `{}`", dest.display()))?;

        Ok(())
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    /// Tear down the execution environment immediately.
    ///
    /// Removes the working directory tree.  After this call the environment
    /// should be considered consumed — subsequent calls are safe but will
    /// return errors because the directory no longer exists.
    pub fn teardown(&mut self) -> Result<()> {
        self.cleanup_on_drop = false;
        if self.working_dir.exists() {
            fs::remove_dir_all(&self.working_dir).with_context(|| {
                format!(
                    "failed to remove exec env working directory `{}`",
                    self.working_dir.display()
                )
            })?;
        }
        Ok(())
    }

    /// Prevent automatic cleanup when this environment is dropped.
    ///
    /// Call this when the caller wants to retain the working directory after
    /// the `ExecEnv` value is no longer needed (e.g. to hand it off to
    /// another process).
    pub fn keep(&mut self) {
        self.cleanup_on_drop = false;
    }
}

impl Drop for ExecEnv {
    fn drop(&mut self) {
        if self.cleanup_on_drop && self.working_dir.exists() {
            let _ = fs::remove_dir_all(&self.working_dir);
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_working_dir_path(_provider: Provider, task_id: &str, base_dir: Option<&Path>) -> PathBuf {
    let base = base_dir.map_or_else(std::env::temp_dir, Path::to_path_buf);

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    // Sanitize the task_id so it is safe to use as a directory name component.
    let safe_task_id: String = task_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    base.join(format!("volva-execenv-{safe_task_id}-{stamp}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{ExecEnv, ExecEnvConfig, GcMetadata, Provider};

    fn unique_base(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_millis();
        std::env::temp_dir().join(format!("volva-execenv-test-{label}-{millis}"))
    }

    // -----------------------------------------------------------------------
    // Directory setup and cleanup
    // -----------------------------------------------------------------------

    #[test]
    fn execenv_creates_working_directory() {
        let base = unique_base("create");
        let config = ExecEnvConfig::new(Provider::Claude, "task-create").with_base_dir(&base);

        let env = ExecEnv::create(config).expect("env should be created");
        assert!(env.working_dir().exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn execenv_cleans_up_on_drop() {
        let base = unique_base("drop");
        let config = ExecEnvConfig::new(Provider::Claude, "task-drop").with_base_dir(&base);

        let working_dir = {
            let env = ExecEnv::create(config).expect("env should be created");
            env.working_dir().to_path_buf()
        };

        assert!(
            !working_dir.exists(),
            "working dir should be removed on drop"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn execenv_teardown_removes_directory() {
        let base = unique_base("teardown");
        let config = ExecEnvConfig::new(Provider::Claude, "task-teardown").with_base_dir(&base);

        let mut env = ExecEnv::create(config).expect("env should be created");
        let working_dir = env.working_dir().to_path_buf();

        env.teardown().expect("teardown should succeed");
        assert!(
            !working_dir.exists(),
            "working dir should be removed after teardown"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn execenv_keep_prevents_cleanup_on_drop() {
        let base = unique_base("keep");
        let config = ExecEnvConfig::new(Provider::Claude, "task-keep").with_base_dir(&base);

        let working_dir = {
            let mut env = ExecEnv::create(config).expect("env should be created");
            env.keep();
            env.working_dir().to_path_buf()
        };

        assert!(
            working_dir.exists(),
            "working dir should persist after drop when keep() was called"
        );
        let _ = fs::remove_dir_all(&base);
    }

    // -----------------------------------------------------------------------
    // Provider config injection
    // -----------------------------------------------------------------------

    #[test]
    fn inject_provider_config_writes_claude_md() {
        let base = unique_base("inject-claude");
        let config =
            ExecEnvConfig::new(Provider::Claude, "task-inject-claude").with_base_dir(&base);

        let env = ExecEnv::create(config).expect("env should be created");

        // Write a temp source file and inject it.
        let src = base.join("source-CLAUDE.md");
        fs::create_dir_all(&base).ok();
        fs::write(&src, "# Test Provider Config\n").expect("source should write");

        env.inject_provider_config(Some(&src))
            .expect("inject should succeed");

        let dest = env.working_dir().join("CLAUDE.md");
        assert!(dest.exists(), "CLAUDE.md should exist after inject");

        let content = fs::read_to_string(&dest).expect("CLAUDE.md should be readable");
        assert!(content.contains("Test Provider Config"));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn inject_provider_config_writes_agents_md_for_codex() {
        let base = unique_base("inject-codex");
        let config = ExecEnvConfig::new(Provider::Codex, "task-inject-codex").with_base_dir(&base);

        let env = ExecEnv::create(config).expect("env should be created");

        let src = base.join("source-AGENTS.md");
        fs::create_dir_all(&base).ok();
        fs::write(&src, "# Codex Config\n").expect("source should write");

        env.inject_provider_config(Some(&src))
            .expect("inject should succeed");

        let dest = env.working_dir().join("AGENTS.md");
        assert!(dest.exists(), "AGENTS.md should exist after inject");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn inject_provider_config_skips_gracefully_when_source_missing() {
        let base = unique_base("inject-missing");
        let config =
            ExecEnvConfig::new(Provider::Claude, "task-inject-missing").with_base_dir(&base);

        let env = ExecEnv::create(config).expect("env should be created");

        // Provide a path that does not exist.
        let nonexistent = base.join("no-such-file.md");
        env.inject_provider_config(Some(&nonexistent))
            .expect("inject with missing source should not fail");

        // Nothing should have been written.
        assert!(!env.working_dir().join("CLAUDE.md").exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn inject_provider_config_skips_gracefully_when_source_is_none() {
        let base = unique_base("inject-none");
        let config = ExecEnvConfig::new(Provider::Claude, "task-inject-none").with_base_dir(&base);

        let env = ExecEnv::create(config).expect("env should be created");
        env.inject_provider_config(None)
            .expect("inject with None source should not fail");

        let _ = fs::remove_dir_all(&base);
    }

    // -----------------------------------------------------------------------
    // Skill injection
    // -----------------------------------------------------------------------

    #[test]
    fn inject_skills_places_files_in_skills_dir() {
        let base = unique_base("skill-inject");
        let skill_src = base.join("skill-source");
        fs::create_dir_all(&skill_src).expect("skill source dir should create");
        fs::write(skill_src.join("my-skill.md"), "# My Skill\n").expect("skill file should write");

        let config = ExecEnvConfig::new(Provider::Claude, "task-skill")
            .with_base_dir(&base)
            .with_skill_source(&skill_src);

        let env = ExecEnv::create(config).expect("env should be created");

        let dest = env.working_dir().join("skills").join("my-skill.md");
        assert!(dest.exists(), "skill file should be present in skills/");

        let content = fs::read_to_string(&dest).expect("skill file should be readable");
        assert!(content.contains("My Skill"));

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn inject_skills_skips_nonexistent_source_dirs() {
        let base = unique_base("skill-missing");
        let config = ExecEnvConfig::new(Provider::Claude, "task-skill-missing")
            .with_base_dir(&base)
            .with_skill_source(base.join("no-such-dir"));

        // Should not error.
        let env = ExecEnv::create(config).expect("env should be created with missing skill src");

        // skills/ dir should not be created if no sources existed.
        assert!(!env.working_dir().join("skills").exists());

        let _ = fs::remove_dir_all(&base);
    }

    // -----------------------------------------------------------------------
    // GC metadata
    // -----------------------------------------------------------------------

    #[test]
    fn gc_metadata_is_written_on_create() {
        let base = unique_base("gc-meta");
        let config = ExecEnvConfig::new(Provider::Claude, "task-gc-meta").with_base_dir(&base);

        let env = ExecEnv::create(config).expect("env should be created");

        let meta_path = env.working_dir().join("gc-metadata.json");
        assert!(meta_path.exists(), "gc-metadata.json should exist");

        let raw = fs::read_to_string(&meta_path).expect("gc-metadata.json should be readable");
        let meta: GcMetadata =
            serde_json::from_str(&raw).expect("gc-metadata.json should deserialize");

        assert_eq!(meta.task_id, "task-gc-meta");
        assert!(
            meta.created_at_secs > 0,
            "created_at_secs should be non-zero"
        );
        assert_eq!(meta.provider, "CLAUDE.md");

        let _ = fs::remove_dir_all(&base);
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    #[test]
    fn execenv_exposes_provider_and_task_id() {
        let base = unique_base("accessors");
        let config = ExecEnvConfig::new(Provider::Gemini, "task-gemini-abc").with_base_dir(&base);

        let env = ExecEnv::create(config).expect("env should be created");

        assert_eq!(env.provider(), Provider::Gemini);
        assert_eq!(env.task_id(), "task-gemini-abc");

        let _ = fs::remove_dir_all(&base);
    }
}
