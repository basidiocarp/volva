use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use volva_config::VolvaConfig;

use crate::BackendRunRequest;

const ENVELOPE_HEADER: &str = "[volva-host-context]";
const MEMORY_PROTOCOL_HEADER: &str = "[hyphae-memory-protocol]";
const USER_PROMPT_HEADER: &str = "[user-prompt]";
const HOST_NOTE: &str = "source: host-provided context from volva";
const HYPHAE_PROTOCOL_COMMAND: &str = "hyphae";
const HYPHAE_PROTOCOL_RESOURCE_URI: &str = "hyphae://protocol/current";
const MEMORY_PROTOCOL_TIMEOUT: Duration = Duration::from_millis(250);
const MEMORY_PROTOCOL_POLL_INTERVAL: Duration = Duration::from_millis(10);

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
pub fn assemble_prompt(config: &VolvaConfig, request: &BackendRunRequest) -> PreparedPrompt {
    let memory_protocol =
        load_memory_protocol_block(request.session.workspace.workspace_root.as_str());
    assemble_prompt_with_memory_protocol(config, request, memory_protocol.as_deref())
}

#[must_use]
pub(crate) fn assemble_prompt_with_memory_protocol(
    config: &VolvaConfig,
    request: &BackendRunRequest,
    memory_protocol: Option<&str>,
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
    let final_prompt = match memory_protocol.filter(|block| !block.trim().is_empty()) {
        Some(block) => format!(
            "{envelope}\n\n{block}\n\n{USER_PROMPT_HEADER}\n{}",
            request.prompt
        ),
        None => format!("{envelope}\n\n{USER_PROMPT_HEADER}\n{}", request.prompt),
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

fn load_memory_protocol_block_from_command(command: &str) -> Option<String> {
    let mut command = Command::new(command);
    command.arg("protocol");

    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;

    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().ok()? {
            if !status.success() {
                return None;
            }

            let output = child.wait_with_output().ok()?;
            let stdout = String::from_utf8(output.stdout).ok()?;
            let surface = serde_json::from_str::<MemoryProtocolSurface>(stdout.trim()).ok()?;
            return Some(format_memory_protocol_block(&surface));
        }

        if start.elapsed() >= MEMORY_PROTOCOL_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }

        thread::sleep(MEMORY_PROTOCOL_POLL_INTERVAL);
    }
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionId,
        ExecutionSessionIdentity, ExecutionSessionState, WorkspaceBinding,
    };

    use crate::BackendRunRequest;

    use super::{assemble_prompt_with_memory_protocol, format_memory_protocol_block};

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

    #[test]
    fn assemble_prompt_prepends_static_host_envelope() {
        let config = VolvaConfig::default();
        let request = BackendRunRequest {
            prompt: "summarize the repository".to_string(),
            session: ExecutionSessionIdentity {
                session_id: ExecutionSessionId("volva-run-test".to_string()),
                mode: ExecutionMode::Run,
                backend: BackendKind::OfficialCli,
                workspace: WorkspaceBinding::from_root("/tmp/project"),
                primary_participant: ExecutionParticipantIdentity {
                    participant_id: "operator@volva".to_string(),
                    host_kind: "volva".to_string(),
                },
                state: ExecutionSessionState::Active,
            },
        };

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
        let request = BackendRunRequest {
            prompt: "hello".to_string(),
            session: ExecutionSessionIdentity {
                session_id: ExecutionSessionId("volva-run-test".to_string()),
                mode: ExecutionMode::Run,
                backend: BackendKind::OfficialCli,
                workspace: WorkspaceBinding::from_root("/tmp/project"),
                primary_participant: ExecutionParticipantIdentity {
                    participant_id: "operator@volva".to_string(),
                    host_kind: "volva".to_string(),
                },
                state: ExecutionSessionState::Active,
            },
        };

        let prepared = assemble_prompt_with_memory_protocol(&config, &request, None);

        assert!(prepared.final_prompt().contains("[user-prompt]\nhello"));
        assert!(!prepared.final_prompt().contains("\nmodel:"));
    }

    #[test]
    fn assemble_prompt_includes_hyphae_memory_protocol_block_when_available() {
        let config = VolvaConfig::default();
        let request = BackendRunRequest {
            prompt: "summarize the repository".to_string(),
            session: ExecutionSessionIdentity {
                session_id: ExecutionSessionId("volva-run-test".to_string()),
                mode: ExecutionMode::Run,
                backend: BackendKind::OfficialCli,
                workspace: WorkspaceBinding::from_root("/tmp/project"),
                primary_participant: ExecutionParticipantIdentity {
                    participant_id: "operator@volva".to_string(),
                    host_kind: "volva".to_string(),
                },
                state: ExecutionSessionState::Active,
            },
        };
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
}
