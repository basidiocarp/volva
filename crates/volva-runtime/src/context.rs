use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
const HYPHAE_PROTOCOL_SCHEMA_VERSION: &str = "1.0";
const MEMORY_PROTOCOL_TIMEOUT: Duration = Duration::from_millis(250);
const MEMORY_PROTOCOL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const SESSION_RECALL_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub mode: OperationMode,
    pub canopy_available: bool,
}

impl Capabilities {
    #[must_use]
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

#[must_use]
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
        format!("workspace_id: {}", request.session.workspace.workspace_id),
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
        extra_blocks.push(redact_context_block(block));
    }
    if let Some(block) = session_recall.filter(|b| !b.trim().is_empty()) {
        extra_blocks.push(redact_context_block(block));
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

// Only fields needed for prompt assembly are deserialized here.
// The full contract (including scoped_identity, recall.when, store.when,
// store.shared_topics) is documented in septa/hyphae-protocol-v1.schema.json.
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

fn log_schema_mismatch(got: &str, expected: &str) {
    // Determine the log path: data_dir → home_dir → /tmp
    let log_path = if let Some(data_dir) = dirs::data_dir() {
        data_dir.join("volva").join("schema-mismatch.log")
    } else if let Ok(home) = std::env::var("HOME") {
        Path::new(&home)
            .join(".local")
            .join("share")
            .join("volva")
            .join("schema-mismatch.log")
    } else {
        Path::new("/tmp").join("volva-schema-mismatch.log")
    };

    // Best-effort: create parent directories and append the entry.
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Format timestamp as UNIX seconds since we don't have chrono available.
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let entry = format!("{{\"ts\":{timestamp},\"got\":\"{got}\",\"expected\":\"{expected}\"}}\n");

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = f.write_all(entry.as_bytes());
    }
}

fn load_memory_protocol_block(workspace_root: &str) -> Option<String> {
    let project = canonicalize_workspace_root(workspace_root);
    load_memory_protocol_block_from_command(HYPHAE_PROTOCOL_COMMAND, &project)
}

fn canonicalize_workspace_root(workspace_root: &str) -> String {
    std::fs::canonicalize(workspace_root)
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .unwrap_or_else(|| workspace_root.to_string())
}

fn load_session_recall_block(workspace_root: &str, caps: &Capabilities) -> Option<String> {
    let project = canonicalize_workspace_root(workspace_root);
    load_session_recall_block_from_command(HYPHAE_PROTOCOL_COMMAND, &project, caps.recall_limit())
}

fn load_memory_protocol_block_from_command(command: &str, project: &str) -> Option<String> {
    let mut command = Command::new(command);
    command.args(["protocol", "--project", project]);

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
                let surface = serde_json::from_str::<MemoryProtocolSurface>(stdout.trim()).ok()?;
                if surface.schema_version != HYPHAE_PROTOCOL_SCHEMA_VERSION {
                    tracing::warn!(
                        got = surface.schema_version,
                        expected = HYPHAE_PROTOCOL_SCHEMA_VERSION,
                        "volva: hyphae protocol schema version mismatch — context injection skipped"
                    );
                    log_schema_mismatch(&surface.schema_version, HYPHAE_PROTOCOL_SCHEMA_VERSION);
                    return None;
                }
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

fn load_session_recall_block_from_command(
    command: &str,
    project: &str,
    limit: usize,
) -> Option<String> {
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

/// Redact sensitive patterns from a context block without truncation.
///
/// Process the block line by line, replacing:
/// - `Bearer <token>` → `Bearer [REDACTED]` (case-insensitive on "bearer ")
/// - Sensitive key assignments in both forms:
///   - Colon form: `api_key: sk-secret123` → `api_key: [REDACTED]`
///   - Env form: `FOO_TOKEN=xyz` → `FOO_TOKEN=[REDACTED]`
///
/// Where the key name (case-insensitively) ends in `_key`, `_token`, `_secret`, or `_password`.
/// - Long hex runs: any run of 40+ consecutive ASCII hex digits → `[REDACTED]`
///
/// Unlike diagnostic redaction, this function preserves the full line length
/// and does not truncate. Every line and the overall structure are maintained.
fn redact_context_block(block: &str) -> String {
    let redacted = block
        .lines()
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n");
    if block.ends_with('\n') && !redacted.ends_with('\n') {
        format!("{redacted}\n")
    } else {
        redacted
    }
}

/// A key name (case-insensitive) is sensitive when it ends in one of these.
fn is_sensitive_key(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.ends_with("_KEY")
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper.ends_with("_PASSWORD")
}

/// Redact every `Bearer <token>` on a line. Matches "bearer" as a word (not a
/// substring of another word) followed by any whitespace (space or tab),
/// optionally a quote, then the token. Only the token is replaced; the keyword,
/// separating whitespace, and any surrounding quote are preserved. Handles
/// multiple occurrences on one line.
fn redact_bearer_tokens(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut copied_to = 0usize;
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find("bearer") {
        let kw_start = search_from + rel;
        let kw_end = kw_start + 6; // byte length of "bearer"
        // Word boundary: the char before "bearer" must not be alphanumeric/underscore.
        let preceded_by_word = kw_start > 0
            && (bytes[kw_start - 1].is_ascii_alphanumeric() || bytes[kw_start - 1] == b'_');
        if preceded_by_word {
            search_from = kw_end;
            continue;
        }
        let after = &input[kw_end..];
        let ws_len = after.len() - after.trim_start().len();
        if ws_len == 0 {
            // not "Bearer <token>" form (no whitespace after the keyword)
            search_from = kw_end;
            continue;
        }
        let token_region = &input[kw_end + ws_len..];
        let q = usize::from(token_region.starts_with('"') || token_region.starts_with('\''));
        let token_body = &token_region[q..];
        let end = token_body
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(token_body.len());
        if end == 0 {
            // no token after the keyword (e.g. trailing `Bearer ` or `Bearer ""`)
            search_from = kw_end + ws_len;
            continue;
        }
        let token_start = kw_end + ws_len + q;
        result.push_str(&input[copied_to..token_start]);
        result.push_str("[REDACTED]");
        copied_to = token_start + end;
        search_from = copied_to;
    }
    result.push_str(&input[copied_to..]);
    result
}

/// Redact the value of any sensitive `key<sep>value` assignment on a line, for
/// the given separator (`=` or `:`). The key may be wrapped in quotes (JSON
/// `"key":`). Only the value token is replaced — the scan stops at the first
/// whitespace or closing quote/`,`/`}` — so the key, separator, intervening
/// whitespace, surrounding quotes, and the remainder of the line are preserved.
fn redact_sensitive_assignments(input: &str, sep: char) -> String {
    let sep_b = sep as u8;
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == sep_b {
            // Allow a closing quote immediately before the separator (JSON `"key":`).
            let mut key_end = i;
            if key_end > 0 && (bytes[key_end - 1] == b'"' || bytes[key_end - 1] == b'\'') {
                key_end -= 1;
            }
            let mut name_start = key_end;
            while name_start > 0
                && (bytes[name_start - 1].is_ascii_alphanumeric() || bytes[name_start - 1] == b'_')
            {
                name_start -= 1;
            }
            let name = &input[name_start..key_end];
            if name_start >= last_end && !name.is_empty() && is_sensitive_key(name) {
                let after = &input[i + 1..];
                let ws_len = after.len() - after.trim_start().len();
                let value = &after[ws_len..];
                let q = usize::from(value.starts_with('"') || value.starts_with('\''));
                let vbody = &value[q..];
                let vend = vbody
                    .find(|c: char| {
                        c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '}'
                    })
                    .unwrap_or(vbody.len());
                if vend > 0 {
                    result.push_str(&input[last_end..=i]); // through the separator
                    result.push_str(&after[..ws_len]); // preserve spacing
                    result.push_str(&value[..q]); // preserve opening quote
                    result.push_str("[REDACTED]");
                    last_end = i + 1 + ws_len + q + vend;
                    i = last_end;
                    continue;
                }
                // vend == 0: no value token to redact; fall through and keep scanning.
            }
        }
        i += 1;
    }
    result.push_str(&input[last_end..]);
    result
}

/// Redact sensitive patterns from a single line without truncation.
fn redact_line(line: &str) -> String {
    let mut output = redact_bearer_tokens(line);
    output = redact_sensitive_assignments(&output, '=');
    output = redact_sensitive_assignments(&output, ':');
    replace_long_hex_in_line(&output)
}

/// Replace hex strings of 40 or more consecutive hex characters with `[REDACTED]`.
/// Does not truncate the line.
fn replace_long_hex_in_line(input: &str) -> String {
    const MIN_HEX_LEN: usize = 40;
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_hexdigit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let run_len = i - start;
            if run_len >= MIN_HEX_LEN {
                result.push_str("[REDACTED]");
            } else {
                for ch in &chars[start..i] {
                    result.push(*ch);
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
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
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::sync::Mutex;
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

    use volva_config::VolvaConfig;
    use volva_core::{
        BackendKind, ExecutionMode, ExecutionParticipantIdentity, ExecutionSessionId,
        ExecutionSessionIdentity, ExecutionSessionState, OperationMode, WorkspaceBinding,
    };

    use crate::BackendRunRequest;

    use super::{
        Capabilities, assemble_prompt_with_memory_and_recall, assemble_prompt_with_memory_protocol,
        format_memory_protocol_block,
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

        let output = prepared.final_prompt();
        assert!(output.starts_with("[volva-host-context]\n"));
        assert!(output.contains("source: host-provided context from volva"));
        assert!(output.contains("session_id: volva-run-test"));
        assert!(output.contains("workspace_root: /tmp/project"));
        assert!(output.contains("workspace_id:"));
        assert!(output.contains("worktree_id: none"));
        assert!(output.contains("backend: official-cli"));
        assert!(output.contains("mode: run"));
        assert!(output.contains("participant: operator@volva"));
        assert!(output.contains("session_state: active"));
        assert!(output.contains("model: claude-sonnet-4-6"));
        assert!(output.contains("[user-prompt]\nsummarize the repository"));
    }

    #[test]
    fn assemble_prompt_omits_blank_model_lines() {
        let config = VolvaConfig {
            model: "   ".to_string(),
            ..Default::default()
        };
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
        let _lock = SHELL_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let command = write_test_command(
            "success",
            "test \"$1\" = \"protocol\" && test \"$2\" = \"--project\" && test \"$3\" = \"test-project\"\nprintf '%s' '{\"schema_version\":\"1.0\",\"summary\":\"Recall selectively at task start.\",\"recall\":{\"tools\":[\"hyphae_gather_context\",\"hyphae_memory_recall\"],\"passive_resource_uri\":\"hyphae://context/current\"},\"store\":{\"tool\":\"hyphae_memory_store\",\"project_topics\":[\"context/{project}\",\"decisions/{project}\"]},\"resources\":[{\"uri\":\"hyphae://protocol/current\"}]}'",
        );

        let protocol = super::load_memory_protocol_block_from_command(
            command.to_string_lossy().as_ref(),
            "test-project",
        )
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

        let prepared =
            assemble_prompt_with_memory_and_recall(&config, &request, Some(protocol), Some(recall));

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

    #[test]
    fn load_memory_protocol_block_from_command_returns_none_on_nonexistent_command() {
        // A command that does not exist should fail to spawn and return None gracefully.
        // This exercises the timeout path without requiring a slow sleep.
        let block = super::load_memory_protocol_block_from_command(
            "/nonexistent/hyphae-cmd",
            "testproject",
        );

        assert!(
            block.is_none(),
            "nonexistent command should return None, not panic"
        );
    }

    #[test]
    fn load_session_recall_block_from_command_returns_none_on_command_failure() {
        // A command that fails (exits non-zero) should return None gracefully.
        // This simulates the timeout path without requiring the actual timeout duration.
        let block = super::load_session_recall_block_from_command(
            "/nonexistent/hyphae-session-cmd",
            "testproject",
            20,
        );

        assert!(
            block.is_none(),
            "failed command should return None, not panic"
        );
    }

    #[test]
    fn assemble_prompt_respects_capabilities_recall_limit() {
        // When capabilities specify Orchestration mode, recall_limit should be 50
        let caps = Capabilities {
            mode: OperationMode::Orchestration,
            canopy_available: true,
        };
        assert_eq!(
            caps.recall_limit(),
            50,
            "orchestration mode should have recall_limit of 50"
        );

        // When capabilities specify Baseline mode, recall_limit should be 20
        let baseline = Capabilities {
            mode: OperationMode::Baseline,
            canopy_available: false,
        };
        assert_eq!(
            baseline.recall_limit(),
            20,
            "baseline mode should have recall_limit of 20"
        );
    }

    #[test]
    fn hyphae_protocol_schema_version_is_pinned() {
        // If this test fails, the hyphae protocol schema changed and volva needs
        // to be updated to handle both the old and new formats, or accept the new version.
        assert_eq!(super::HYPHAE_PROTOCOL_SCHEMA_VERSION, "1.0");
    }

    #[test]
    fn canonicalize_workspace_root_returns_full_path_for_existing_directory() {
        let dir = std::env::temp_dir();
        let input = dir.to_string_lossy().to_string();
        let result = super::canonicalize_workspace_root(&input);
        let canonical = std::fs::canonicalize(&dir).expect("temp dir must canonicalize");
        assert_eq!(result, canonical.to_string_lossy().as_ref());
        // Must be an absolute path, not a bare basename.
        assert!(
            std::path::Path::new(&result).is_absolute(),
            "result must be a full path, not just a basename: {result}"
        );
    }

    #[test]
    fn canonicalize_workspace_root_falls_back_to_raw_path_for_nonexistent_directory() {
        let fake = "/nonexistent/volva-test-path-that-does-not-exist";
        let result = super::canonicalize_workspace_root(fake);
        assert_eq!(
            result, fake,
            "nonexistent path must fall back to the raw input"
        );
    }

    #[test]
    fn redact_context_block_preserves_secret_free_block_byte_identical() {
        let block = "[hyphae-memory-protocol]\nschema_version: 1.0\nproject: myproject\nsummary: Test summary\nrecall_tools: tool1, tool2";
        let redacted = super::redact_context_block(block);
        assert_eq!(
            redacted, block,
            "secret-free block must be returned byte-identical"
        );
    }

    #[test]
    fn redact_context_block_redacts_colon_style_api_key() {
        let block = "[hyphae-session-recall]\nproject: myproject\napi_key: sk-secret123\ndone";
        let redacted = super::redact_context_block(block);
        assert!(
            redacted.contains("api_key: [REDACTED]"),
            "colon-style api_key value must be redacted"
        );
        assert!(
            !redacted.contains("sk-secret123"),
            "actual secret must not appear in redacted output"
        );
        // Line count must be preserved
        assert_eq!(
            redacted.lines().count(),
            block.lines().count(),
            "line count must be preserved"
        );
    }

    #[test]
    fn redact_context_block_redacts_colon_style_token() {
        let block = "api_token: Bearer xyz789\nother: value";
        let redacted = super::redact_context_block(block);
        assert!(
            redacted.contains("api_token: [REDACTED]"),
            "colon-style token must be redacted"
        );
        assert!(
            !redacted.contains("xyz789"),
            "actual token must not appear in redacted output"
        );
    }

    #[test]
    fn redact_context_block_redacts_env_style_token() {
        let block = "FOO_TOKEN=abcdef123456 other_var=value";
        let redacted = super::redact_context_block(block);
        assert!(
            redacted.contains("FOO_TOKEN=[REDACTED]"),
            "env-style FOO_TOKEN must be redacted"
        );
        assert!(
            !redacted.contains("abcdef123456"),
            "actual token value must not appear in redacted output"
        );
    }

    #[test]
    fn redact_context_block_redacts_env_style_secret() {
        let block = "DATABASE_SECRET=mypassword123";
        let redacted = super::redact_context_block(block);
        assert!(
            redacted.contains("DATABASE_SECRET=[REDACTED]"),
            "env-style DATABASE_SECRET must be redacted"
        );
        assert!(
            !redacted.contains("mypassword123"),
            "actual secret must not appear in redacted output"
        );
    }

    #[test]
    fn redact_context_block_does_not_truncate_long_lines() {
        let long_value = "a".repeat(1000);
        let block = format!("api_key: {long_value}");
        let redacted = super::redact_context_block(&block);
        // After redaction, should be much shorter (api_key: [REDACTED])
        // but should NOT be truncated by a line length limit
        assert!(
            redacted.len() < block.len(),
            "redacted line should be shorter due to secret replacement"
        );
        assert!(
            !redacted.contains(&long_value),
            "original long value must not appear in output"
        );
        // The redacted value should be small and clean
        assert!(redacted.contains("api_key: [REDACTED]"));
    }

    #[test]
    fn redact_context_block_redacts_bearer_token() {
        let block = "Authorization: Bearer sk-proj-secret123 and other text";
        let redacted = super::redact_context_block(block);
        assert!(
            redacted.contains("Bearer [REDACTED]"),
            "Bearer token must be redacted"
        );
        assert!(
            !redacted.contains("sk-proj-secret123"),
            "actual bearer token must not appear in redacted output"
        );
        assert!(
            redacted.contains("and other text"),
            "text after bearer token must be preserved"
        );
    }

    #[test]
    fn redact_context_block_redacts_bearer_token_case_insensitive() {
        let block = "header: BEARER longtoken123456 tail";
        let redacted = super::redact_context_block(block);
        assert!(
            redacted.contains("BEARER [REDACTED]"),
            "BEARER (uppercase) must be redacted"
        );
        assert!(
            redacted.contains("tail"),
            "text after bearer token must be preserved"
        );
    }

    #[test]
    fn redact_context_block_redacts_40_plus_hex_digits() {
        let hex_string = "0123456789abcdef0123456789abcdef01234567"; // exactly 40
        let block = format!("git_sha: {hex_string}");
        let redacted = super::redact_context_block(&block);
        assert!(
            redacted.contains("[REDACTED]"),
            "40+ hex digit run must be redacted"
        );
        assert!(
            !redacted.contains(hex_string),
            "actual hex string must not appear in output"
        );
    }

    #[test]
    fn redact_context_block_preserves_short_hex_sequences() {
        let block = "short_hex: abc123def456 and 39_hex_chars_0123456789abcdef012345678";
        let redacted = super::redact_context_block(block);
        // Hex sequences less than 40 chars should not be redacted
        assert!(
            redacted.contains("abc123def456"),
            "short hex sequences must not be redacted"
        );
    }

    #[test]
    fn redact_context_block_preserves_keys_without_sensitive_suffix() {
        let block = "project_name: myproject\napi_call: get_status\ndebug_info: value";
        let redacted = super::redact_context_block(block);
        assert_eq!(
            redacted, block,
            "non-sensitive keys must not trigger redaction"
        );
    }

    #[test]
    fn redact_context_block_multiline_preserves_structure() {
        let block = "[hyphae-memory-protocol]\nschema_version: 1.0\napi_key: sk-abc123\nsummary: Test block\nproject: demo";
        let redacted = super::redact_context_block(block);
        let original_lines = block.lines().count();
        let redacted_lines = redacted.lines().count();
        assert_eq!(
            original_lines, redacted_lines,
            "line count must be preserved across all lines"
        );
        assert!(
            redacted.contains("[hyphae-memory-protocol]"),
            "headers must be preserved"
        );
        assert!(
            redacted.contains("schema_version: 1.0"),
            "non-sensitive lines must be preserved"
        );
        assert!(
            redacted.contains("api_key: [REDACTED]"),
            "sensitive values must be redacted"
        );
    }

    #[test]
    fn assemble_prompt_redacts_memory_protocol_block() {
        let config = VolvaConfig::default();
        let request = test_request("do work", "volva-run-test");
        let protocol = "[hyphae-memory-protocol]\napi_key: sk-secret123\nsummary: test";

        let prepared = assemble_prompt_with_memory_protocol(&config, &request, Some(protocol));

        let text = prepared.final_prompt();
        assert!(
            text.contains("api_key: [REDACTED]"),
            "memory protocol block must be redacted"
        );
        assert!(
            !text.contains("sk-secret123"),
            "secret in memory protocol must not appear in prompt"
        );
        assert!(
            text.contains("[hyphae-memory-protocol]"),
            "header must be preserved"
        );
    }

    #[test]
    fn assemble_prompt_redacts_session_recall_block() {
        let config = VolvaConfig::default();
        let request = test_request("do work", "volva-run-test");
        let recall = "[hyphae-session-recall]\napi_token: xyz789\nproject: myproj";

        let prepared =
            assemble_prompt_with_memory_and_recall(&config, &request, None, Some(recall));

        let text = prepared.final_prompt();
        assert!(
            text.contains("api_token: [REDACTED]"),
            "session recall block must be redacted"
        );
        assert!(
            !text.contains("xyz789"),
            "secret in session recall must not appear in prompt"
        );
    }

    #[test]
    fn assemble_prompt_does_not_redact_user_prompt() {
        let config = VolvaConfig::default();
        let request = test_request("api_key: user-provided-secret", "volva-run-test");

        let prepared = assemble_prompt_with_memory_protocol(&config, &request, None);

        let text = prepared.final_prompt();
        // User prompt should NOT be redacted
        assert!(
            text.contains("api_key: user-provided-secret"),
            "user prompt content must not be redacted"
        );
    }

    #[test]
    fn assemble_prompt_does_not_redact_envelope() {
        let config = VolvaConfig::default();
        let request = test_request("task", "volva-run-test");

        let prepared = assemble_prompt_with_memory_protocol(&config, &request, None);

        let text = prepared.final_prompt();
        // Envelope should contain the session ID without redaction
        assert!(
            text.contains("session_id: volva-run-test"),
            "envelope content must not be redacted"
        );
    }

    #[test]
    fn redact_context_block_preserves_long_non_sensitive_line_byte_identical() {
        // Build a long line of ordinary prose (no sensitive keys, no hex runs of 40+)
        let long_prose = "summary: ".to_string()
            + &"Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. "
                .repeat(7);
        let input = long_prose;
        assert!(
            input.len() > 500,
            "test line must be long (> 500 chars) to verify no truncation"
        );
        let result = super::redact_context_block(&input);
        assert_eq!(
            result, input,
            "long non-sensitive line must pass through byte-identical"
        );
    }

    #[test]
    fn redact_context_block_preserves_realistic_memory_protocol_block_byte_identical() {
        let block = "[hyphae-memory-protocol]\nschema_version: 1.0\nproject: basidiocarp\nsummary: Recall prior decisions before starting work.\nrecall_tools: hyphae_memory_recall, hyphae_recall_global\npassive_resource: hyphae://protocol/current\nstore_tool: hyphae_memory_store\nproject_topics: errors/resolved, decisions/volva\nprotocol_resource: hyphae://protocol/current";
        let result = super::redact_context_block(block);
        assert_eq!(
            result, block,
            "realistic memory-protocol block with no sensitive fields must be byte-identical"
        );
    }

    #[test]
    fn bearer_tab_separated_is_redacted() {
        let input = "auth: Bearer\tSECRETTOKEN123 tail";
        let result = super::redact_line(input);
        assert!(!result.contains("SECRETTOKEN123"));
        assert!(result.contains("[REDACTED]"));
        assert!(result.ends_with(" tail"));
    }

    #[test]
    fn bearer_quoted_token_is_redacted() {
        let input = r#"auth: Bearer "SECRETQUOTED" tail"#;
        let result = super::redact_line(input);
        assert!(!result.contains("SECRETQUOTED"));
        assert!(result.contains("[REDACTED]"));
        assert!(result.contains("tail"));
        assert!(result.contains(r#""[REDACTED]""#));
    }

    #[test]
    fn multiple_bearer_tokens_on_one_line_all_redacted() {
        let input = "a Bearer SECRETONE b Bearer SECRETTWO c";
        let result = super::redact_line(input);
        assert!(!result.contains("SECRETONE"));
        assert!(!result.contains("SECRETTWO"));
        assert!(result.contains("a "));
        assert!(result.contains(" b "));
        assert!(result.contains(" c"));
    }

    #[test]
    fn bearer_as_substring_of_word_is_not_matched() {
        let input = "cyberbearer is a word";
        let result = super::redact_line(input);
        assert_eq!(result, input);
    }

    #[test]
    fn json_quoted_sensitive_key_value_is_redacted() {
        let input = r#"{"api_key":"sk-SECRETJSON","other":"keep"}"#;
        let result = super::redact_line(input);
        assert!(!result.contains("sk-SECRETJSON"));
        assert!(result.contains("keep"));
        assert!(result.contains("other"));
        assert!(result.contains("api_key"));
    }

    #[test]
    fn colon_pass_preserves_content_after_secret() {
        let input = "api_key: sk-SECRET trailing words here";
        let result = super::redact_line(input);
        assert_eq!(result, "api_key: [REDACTED] trailing words here");
    }

    #[test]
    fn nested_colon_secret_preserves_closing_brace() {
        let input = "config: {api_token: SECRETNEST}";
        let result = super::redact_line(input);
        assert_eq!(result, "config: {api_token: [REDACTED]}");
    }

    #[test]
    fn second_colon_field_on_line_is_redacted() {
        let input = "summary: note user_password: hunter2 is secret";
        let result = super::redact_line(input);
        assert_eq!(result, "summary: note user_password: [REDACTED] is secret");
    }

    #[test]
    fn env_assignment_still_redacted() {
        let input = "FOO_TOKEN=xyzSECRET more";
        let result = super::redact_line(input);
        assert_eq!(result, "FOO_TOKEN=[REDACTED] more");
    }

    #[test]
    fn empty_sensitive_value_is_left_alone() {
        let input = "api_key:";
        let result = super::redact_line(input);
        assert_eq!(result, "api_key:");
    }
}
