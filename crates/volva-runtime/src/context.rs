use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tracing::warn;
use volva_config::VolvaConfig;
use volva_core::OperationMode;

use crate::BackendRunRequest;

const ENVELOPE_HEADER: &str = "[volva-host-context]";
const MEMORY_PROTOCOL_HEADER: &str = "[hyphae-memory-protocol]";
const SESSION_RECALL_HEADER: &str = "[hyphae-session-recall]";
const USER_PROMPT_HEADER: &str = "[user-prompt]";
const HOST_NOTE: &str = "source: host-provided context from volva";
const HYPHAE_PROTOCOL_COMMAND: &str = "hyphae";
const HYPHAE_PROTOCOL_RESOURCE_URI: &str = "hyphae://protocol/current";
const MEMORY_PROTOCOL_TIMEOUT: Duration = Duration::from_millis(250);
const MEMORY_PROTOCOL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const SESSION_RECALL_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub mode: OperationMode,
    pub canopy_available: bool,
}

impl Capabilities {
    pub fn recall_limit(&self) -> usize {
        match self.mode {
            OperationMode::Baseline => 20,
            OperationMode::Orchestration => 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPrompt {
    final_prompt: String,
}

impl PreparedPrompt {
    #[must_use]
    pub fn final_prompt(&self) -> &str {
        &self.final_prompt
    }
}

#[must_use]
pub fn assemble_prompt(
    config: &VolvaConfig,
    request: &BackendRunRequest,
    caps: &Capabilities,
) -> PreparedPrompt {
    let workspace_root = request.session.workspace.workspace_root.as_str();
    let memory_protocol = load_memory_protocol_block(workspace_root);
    let session_recall = load_session_recall_block(workspace_root, caps);
    assemble_prompt_with_memory_and_recall(
        config,
        request,
        memory_protocol.as_deref(),
        session_recall.as_deref(),
    )
}

#[must_use]
#[cfg(test)]
pub(crate) fn assemble_prompt_with_memory_protocol(
    config: &VolvaConfig,
    request: &BackendRunRequest,
    memory_protocol: Option<&str>,
) -> PreparedPrompt {
    assemble_prompt_with_memory_and_recall(config, request, memory_protocol, None)
}

pub fn capabilities_baseline() -> Capabilities {
    Capabilities {
        mode: OperationMode::Baseline,
        canopy_available: false,
    }
}

#[must_use]
pub(crate) fn assemble_prompt_with_memory_and_recall(
    config: &VolvaConfig,
    request: &BackendRunRequest,
    memory_protocol: Option<&str>,
    session_recall: Option<&str>,
) -> PreparedPrompt {
    let mut lines = vec![
        ENVELOPE_HEADER.to_string(),
        HOST_NOTE.to_string(),
        format!("session_id: {}", request.session.session_id),
        format!(
            "workspace_root: {}",
            request.session.workspace.workspace_root
        ),
        format!(
            "worktree_id: {}",
            request
                .session
                .workspace
                .worktree_id
                .as_deref()
                .unwrap_or("none")
        ),
        format!("backend: {}", request.session.backend),
        format!("mode: {}", request.session.mode),
        format!(
            "participant: {}",
            request.session.primary_participant.participant_id
        ),
        format!("session_state: {}", request.session.state),
    ];

    if !config.model.trim().is_empty() {
        lines.push(format!("model: {}", config.model.trim()));
    }

    let envelope = lines.join("\n");

    // Collect optional context blocks between the envelope and user prompt.
    let mut extra_blocks = Vec::new();
    if let Some(block) = memory_protocol.filter(|b| !b.trim().is_empty()) {
        extra_blocks.push(block.to_string());
    }
    if let Some(block) = session_recall.filter(|b| !b.trim().is_empty()) {
        extra_blocks.push(block.to_string());
    }

    let final_prompt = if extra_blocks.is_empty() {
        format!("{envelope}\n\n{USER_PROMPT_HEADER}\n{}", request.prompt)
    } else {
        let blocks = extra_blocks.join("\n\n");
        format!(
            "{envelope}\n\n{blocks}\n\n{USER_PROMPT_HEADER}\n{}",
            request.prompt
        )
    };

    PreparedPrompt { final_prompt }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct MemoryProtocolSurface {
    schema_version: String,
    #[serde(default)]
    project: Option<String>,
    summary: String,
    recall: RecallPhase,
    store: StorePhase,
    #[serde(default)]
    resources: Vec<ProtocolResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct RecallPhase {
    #[serde(default)]
    tools: Vec<String>,
    passive_resource_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct StorePhase {
    tool: String,
    #[serde(default)]
    project_topics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ProtocolResource {
    uri: String,
}

fn load_memory_protocol_block(workspace_root: &str) -> Option<String> {
    let _ = workspace_root;
    load_memory_protocol_block_from_command(HYPHAE_PROTOCOL_COMMAND)
}

fn load_session_recall_block(workspace_root: &str, caps: &Capabilities) -> Option<String> {
    let project = Path::new(workspace_root)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(workspace_root);
    load_session_recall_block_from_command(HYPHAE_PROTOCOL_COMMAND, project, caps.recall_limit())
}

fn load_memory_protocol_block_from_command(command: &str) -> Option<String> {
    let mut command = Command::new(command);
    command.arg("protocol");

    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;

    // Take stdout now so we can read it after try_wait() confirms exit.
    // This avoids the double-wait bug: try_wait() reaps the exit status, then
    // wait_with_output() would attempt a second wait on an already-reaped child.
    let mut stdout_handle = child.stdout.take()?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                // Read stdout using the handle we took before the poll loop.
                // The child has already exited so this will not block.
                let mut stdout_bytes = Vec::new();
                stdout_handle.read_to_end(&mut stdout_bytes).ok()?;
                let stdout = String::from_utf8(stdout_bytes).ok()?;
                let surface =
                    serde_json::from_str::<MemoryProtocolSurface>(stdout.trim()).ok()?;
                return Some(format_memory_protocol_block(&surface));
            }
            Ok(None) => {}
            Err(err) => {
                warn!(error = %err, "try_wait failed while polling memory protocol child");
                return None;
            }
        }

        if start.elapsed() >= MEMORY_PROTOCOL_TIMEOUT {
            tracing::warn!(
                timeout_ms = MEMORY_PROTOCOL_TIMEOUT.as_millis(),
                "volva: hyphae protocol load timed out — session starts without memory context"
            );
            let _ = child.kill();
            // Reap the child to avoid a zombie; discard the exit status.
            if let Err(err) = child.wait() {
                warn!(error = %err, "wait failed after killing memory protocol child");
            }
            return None;
        }

        thread::sleep(MEMORY_PROTOCOL_POLL_INTERVAL);
    }
}

fn load_session_recall_block_from_command(command: &str, project: &str, limit: usize) -> Option<String> {
    let limit_str = limit.to_string();
    let mut child = Command::new(command)
        .args([
            "session",
            "context",
            "--project",
            project,
            "--limit",
            &limit_str,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;

    // Take stdout before entering the poll loop for the same reason as above:
    // try_wait() reaps the exit status; reading via a separate handle avoids
    // the double-wait that wait_with_output() would cause.
    let mut stdout_handle = child.stdout.take()?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut stdout_bytes = Vec::new();
                stdout_handle.read_to_end(&mut stdout_bytes).ok()?;
                let stdout = String::from_utf8(stdout_bytes).ok()?;
                let trimmed = stdout.trim().to_string();
                if trimmed.is_empty() {
                    return None;
                }
                return Some(format_session_recall_block(project, &trimmed));
            }
            Ok(None) => {}
            Err(err) => {
                warn!(error = %err, "try_wait failed while polling session recall child");
                return None;
            }
        }

        if start.elapsed() >= SESSION_RECALL_TIMEOUT {
            tracing::warn!(
                timeout_ms = SESSION_RECALL_TIMEOUT.as_millis(),
                "volva: hyphae session recall timed out — prompt sent without session context"
            );
            let _ = child.kill();
            // Reap the child to avoid a zombie; discard the exit status.
            if let Err(err) = child.wait() {
                warn!(error = %err, "wait failed after killing session recall child");
            }
            return None;
        }

        thread::sleep(MEMORY_PROTOCOL_POLL_INTERVAL);
    }
}

fn format_session_recall_block(project: &str, raw_output: &str) -> String {
    let mut lines = vec![
        SESSION_RECALL_HEADER.to_string(),
        format!("project: {project}"),
    ];
    for line in raw_output.lines() {
        lines.push(line.to_string());
    }
    lines.join("\n")
}

fn format_memory_protocol_block(surface: &MemoryProtocolSurface) -> String {
    let recall_tools = if surface.recall.tools.is_empty() {
        "none".to_string()
    } else {
        surface.recall.tools.join(", ")
    };
    let project_topics = if surface.store.project_topics.is_empty() {
        "none".to_string()
    } else {
        surface.store.project_topics.join(", ")
    };
    let protocol_resource = surface
        .resources
        .iter()
        .find(|resource| resource.uri == HYPHAE_PROTOCOL_RESOURCE_URI)
        .map(|resource| resource.uri.as_str());

    let mut lines = vec![
        MEMORY_PROTOCOL_HEADER.to_string(),
        format!("schema_version: {}", surface.schema_version),
        format!("project: {}", surface.project.as_deref().unwrap_or("none")),
        format!("summary: {}", surface.summary),
        format!("recall_tools: {recall_tools}"),
        format!("passive_resource: {}", surface.recall.passive_resource_uri),
        format!("store_tool: {}", surface.store.tool),
        format!("project_topics: {project_topics}"),
    ];
    if let Some(protocol_resource) = protocol_resource {
        lines.push(format!("protocol_resource: {protocol_resource}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionId,
        ExecutionSessionIdentity, ExecutionSessionState, OperationMode, WorkspaceBinding,
    };

    use crate::BackendRunRequest;

    use super::{
        assemble_prompt_with_memory_and_recall, assemble_prompt_with_memory_protocol,
        format_memory_protocol_block, Capabilities,
    };

    // Shell-subprocess tests must not run concurrently: parallel spawns on macOS
    // can exhaust the 250 ms / 500 ms timeouts embedded in the production poll loop.
    // Acquire this lock at the top of every test that calls a `load_*_from_command`
    // helper via a real shell script.
    #[cfg(unix)]
    static SHELL_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(unix)]
    fn unique_temp_path(label: &str) -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_millis();
        std::env::temp_dir().join(format!("volva-memory-protocol-{label}-{millis}.sh"))
    }

    #[cfg(unix)]
    fn write_test_command(label: &str, body: &str) -> PathBuf {
        let path = unique_temp_path(label);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n"))
            .expect("test command script should write");
        let mut permissions = fs::metadata(&path)
            .expect("test command metadata should exist")
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o700);
        }
        fs::set_permissions(&path, permissions).expect("test command script should be executable");
        path
    }

    fn test_request(prompt: &str, session_id: &str) -> BackendRunRequest {
        BackendRunRequest {
            prompt: prompt.to_string(),
            session: ExecutionSessionIdentity {
                session_id: ExecutionSessionId(session_id.to_string()),
                mode: ExecutionMode::Run,
                backend: BackendKind::OfficialCli,
                workspace: WorkspaceBinding::from_root("/tmp/project"),
                primary_participant: ExecutionParticipantIdentity {
                    participant_id: "operator@volva".to_string(),
                    host_kind: "volva".to_string(),
                },
                state: ExecutionSessionState::Active,
            },
            capabilities: Capabilities {
                mode: OperationMode::Baseline,
                canopy_available: false,
            },
        }
    }

    #[test]
    fn assemble_prompt_prepends_static_host_envelope() {
        let config = VolvaConfig::default();
        let request = test_request("summarize the repository", "volva-run-test");

        let prepared = assemble_prompt_with_memory_protocol(&config, &request, None);

        assert_eq!(
            prepared.final_prompt(),
            "[volva-host-context]\n\
source: host-provided context from volva\n\
session_id: volva-run-test\n\
workspace_root: /tmp/project\n\
worktree_id: none\n\
backend: official-cli\n\
mode: run\n\
participant: operator@volva\n\
session_state: active\n\
model: claude-sonnet-4-6\n\n\
[user-prompt]\n\
summarize the repository"
        );
    }

    #[test]
    fn assemble_prompt_omits_blank_model_lines() {
        let mut config = VolvaConfig::default();
        config.model = "   ".to_string();
        let request = test_request("hello", "volva-run-test");

        let prepared = assemble_prompt_with_memory_protocol(&config, &request, None);

        assert!(prepared.final_prompt().contains("[user-prompt]\nhello"));
        assert!(!prepared.final_prompt().contains("\nmodel:"));
    }

    #[test]
    fn assemble_prompt_includes_hyphae_memory_protocol_block_when_available() {
        let config = VolvaConfig::default();
        let request = test_request("summarize the repository", "volva-run-test");
        let protocol = "[hyphae-memory-protocol]\nsummary: test protocol";

        let prepared = assemble_prompt_with_memory_protocol(&config, &request, Some(protocol));

        assert!(prepared.final_prompt().contains(protocol));
        assert!(prepared.final_prompt().contains("\n\n[user-prompt]\n"));
    }

    #[test]
    fn hyphae_memory_protocol_block_is_concise_and_project_aware() {
        let protocol = format_memory_protocol_block(&super::MemoryProtocolSurface {
            schema_version: "1.0".to_string(),
            project: Some("demo".to_string()),
            summary: "Recall selectively at task start.".to_string(),
            recall: super::RecallPhase {
                tools: vec![
                    "hyphae_gather_context".to_string(),
                    "hyphae_memory_recall".to_string(),
                ],
                passive_resource_uri: "hyphae://context/current".to_string(),
            },
            store: super::StorePhase {
                tool: "hyphae_memory_store".to_string(),
                project_topics: vec!["context/demo".to_string(), "decisions/demo".to_string()],
            },
            resources: vec![super::ProtocolResource {
                uri: "hyphae://protocol/current".to_string(),
            }],
        });

        assert!(protocol.starts_with("[hyphae-memory-protocol]"));
        assert!(protocol.contains("schema_version: 1.0"));
        assert!(protocol.contains("project: demo"));
        assert!(protocol.contains("recall_tools: hyphae_gather_context, hyphae_memory_recall"));
        assert!(protocol.contains("store_tool: hyphae_memory_store"));
        assert!(protocol.contains("protocol_resource: hyphae://protocol/current"));
    }

    #[test]
    fn hyphae_memory_protocol_block_omits_unadvertised_protocol_resource() {
        let protocol = format_memory_protocol_block(&super::MemoryProtocolSurface {
            schema_version: "1.0".to_string(),
            project: None,
            summary: "Recall selectively at task start.".to_string(),
            recall: super::RecallPhase {
                tools: vec!["hyphae_gather_context".to_string()],
                passive_resource_uri: "hyphae://context/current".to_string(),
            },
            store: super::StorePhase {
                tool: "hyphae_memory_store".to_string(),
                project_topics: vec!["context/{project}".to_string()],
            },
            resources: Vec::new(),
        });

        assert!(protocol.contains("project: none"));
        assert!(!protocol.contains("protocol_resource:"));
    }

    #[cfg(unix)]
    #[test]
    fn load_memory_protocol_block_from_command_reads_runtime_surface() {
        let _lock = SHELL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let command = write_test_command(
            "success",
            "test \"$1\" = \"protocol\"\nprintf '%s' '{\"schema_version\":\"1.0\",\"summary\":\"Recall selectively at task start.\",\"recall\":{\"tools\":[\"hyphae_gather_context\",\"hyphae_memory_recall\"],\"passive_resource_uri\":\"hyphae://context/current\"},\"store\":{\"tool\":\"hyphae_memory_store\",\"project_topics\":[\"context/{project}\",\"decisions/{project}\"]},\"resources\":[{\"uri\":\"hyphae://protocol/current\"}]}'",
        );

        let protocol =
            super::load_memory_protocol_block_from_command(command.to_string_lossy().as_ref())
                .expect("protocol command should be parsed");

        assert!(protocol.starts_with("[hyphae-memory-protocol]"));
        assert!(protocol.contains("project: none"));
        assert!(protocol.contains("protocol_resource: hyphae://protocol/current"));

        let _ = fs::remove_file(command);
    }

    #[test]
    fn format_session_recall_block_produces_correct_output() {
        let block = super::format_session_recall_block(
            "myproject",
            "ses_abc [completed] project_root=/tmp/myproject -> did some work\nses_def [active] project_root=/tmp/myproject -> in progress",
        );

        assert!(block.starts_with("[hyphae-session-recall]"));
        assert!(block.contains("project: myproject"));
        assert!(block.contains("ses_abc [completed]"));
        assert!(block.contains("ses_def [active]"));
    }

    #[test]
    fn load_session_recall_block_from_command_returns_none_for_nonexistent_command() {
        // A command that does not exist should fail to spawn and return None gracefully.
        let block = super::load_session_recall_block_from_command(
            "/nonexistent/hyphae-test-binary",
            "myproject",
            20,
        );

        assert!(block.is_none(), "missing command should produce None");
    }

    #[test]
    fn assemble_prompt_includes_both_protocol_and_recall_blocks() {
        let config = VolvaConfig::default();
        let request = test_request("do work", "volva-run-test");
        let protocol = "[hyphae-memory-protocol]\nsummary: test protocol";
        let recall = "[hyphae-session-recall]\nproject: project\nses_abc [completed] -> did work";

        let prepared = assemble_prompt_with_memory_and_recall(
            &config,
            &request,
            Some(protocol),
            Some(recall),
        );

        let text = prepared.final_prompt();
        assert!(text.contains(protocol));
        assert!(text.contains(recall));
        assert!(text.contains("\n\n[user-prompt]\n"));
        // Protocol block should appear before recall block
        let protocol_pos = text.find(protocol).expect("protocol block must be present");
        let recall_pos = text.find(recall).expect("recall block must be present");
        assert!(
            protocol_pos < recall_pos,
            "protocol block should precede recall block"
        );
    }

    #[test]
    fn assemble_prompt_with_recall_only_omits_protocol_block() {
        let config = VolvaConfig::default();
        let request = test_request("do work", "volva-run-test");
        let recall = "[hyphae-session-recall]\nproject: project\nses_abc [completed] -> did work";

        let prepared =
            assemble_prompt_with_memory_and_recall(&config, &request, None, Some(recall));

        let text = prepared.final_prompt();
        assert!(text.contains(recall));
        assert!(!text.contains("[hyphae-memory-protocol]"));
        assert!(text.contains("\n\n[user-prompt]\n"));
    }
}
