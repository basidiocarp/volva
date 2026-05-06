# Workspace-Session Binding Model

## Status
Proposed

## Context

Volva manages execution-host sessions but has no formal model for how sessions bind to workspaces. When canopy begins coordinating multiple agents across multiple workspaces — especially as the ecosystem moves toward workspace-affinity task routing — there is no defined answer for whether a workspace can have multiple concurrent sessions, which state is scoped to the workspace versus the session, or how session transitions are handled cleanly.

The current implementation threads a `WorkspaceBinding` through each `ExecutionSessionIdentity`, but without explicit cardinality, scoping, and switch semantics, the behavior becomes ambiguous when multiple projects are active simultaneously or when an agent moves between workspaces mid-session.

## Decisions

### Cardinality: 1:1 with Override

**Decision**: One active session per workspace at a time (1:1 default).

When a workspace already has an active session, a new session start request is rejected with a clear error message. The session cannot be automatically queued or replaced.

Override is possible via explicit configuration flag (`allow_concurrent_workspace_sessions`), but it is not the default. Operators must opt in to multi-session-per-workspace behavior.

**Rationale**:

- Simpler reasoning about state coherence: a single agent owns the workspace context at any given time.
- Aligns with the single-agent-per-workspace mental model that canopy uses today.
- Avoids resource contention over shared workspace state (working directory, vendor paths, config).
- Provides a natural backpressure signal: if a workspace is occupied, the caller knows to wait or route to a different workspace.
- Override capability exists for future multi-workspace deployments where parallel work on different features in the same workspace makes sense, but that is deferred.

### Scoping: Workspace-Level and Session-Level

**Decision**: Split state into workspace-scoped (persistent across sessions in the same workspace) and session-scoped (per-session, not carried forward):

**Workspace-scoped (shared across sessions):**
- Working directory and canonical workspace root path
- `volva.json` configuration (backend, model, API base, hook adapters)
- Vendor directory path and its contents
- Active backend command and resolved backend config
- Environment variable overrides that apply workspace-wide

**Session-scoped (isolated per session, recreated on start):**
- Session ID (`ExecutionSessionId`)
- Context envelope (recall injection, host context assembly)
- Chat history and conversation state (if persisted)
- Cost counters and token accounting
- Hook event sequence and execution timeline
- Session state machine (Planned → Active → Paused/Resumed → Finished)
- Primary participant identity (the agent or operator running this session)

**Rationale**:

- Workspace config is expensive to recompute and should be validated once per workspace, not per session.
- Vendor artifacts (dependencies, artifacts) are workspace-resident and should be reused across sessions without duplication.
- Session identity must be unique per run, not reused across workspace sessions.
- Cost and history tracking must reset between sessions to prevent incorrect rollup or unintended context carryover.
- Hook events are logically tied to a single execution run and should not be mixed across sessions.

### Workspace Switch Semantics: Clean Handoff with No Carryover

**Decision**: A running session cannot switch workspaces mid-execution.

If a session needs to move to a different workspace (e.g., an agent is reassigned to a new project), the old session must be explicitly ended first:

1. Emit a `session_end` hook with outcome and final state.
2. Persist the final session snapshot to the old workspace's vendor directory.
3. Close all resource handles tied to the old workspace (working directory, config, hook adapters).
4. Start a new session in the target workspace (new session ID, fresh context envelope, no state carryover).

Nothing from the old session is retained in the new one. Cost counters, history, and participant identity start fresh.

**Rationale**:

- Prevents implicit state bleed between workspaces (e.g., chat history from project A appearing in project B).
- Makes workspace transitions visible and auditable via hook events.
- Aligns with the session-first design: sessions are bounded, and boundaries are explicit.
- Simplifies state cleanup: a closed session has a clear point where resources are released.
- Prepares the foundation for future workspace-affinity routing in canopy without retrofitting semantics later.

### Identity: Canonical Absolute Path

**Decision**: A workspace is identified by its canonical absolute path, resolved via `std::fs::canonicalize` at session start.

The workspace root is stored in `WorkspaceBinding.workspace_root` as an absolute path string. Symlinks and relative paths are resolved to a canonical form before any session is created. If two paths resolve to the same canonical path, they refer to the same workspace.

A stable UUID-based workspace ID (independent of path) is desirable for future integration with spore workspace records, but that requires workspace discovery and stable spore workspace registration. For now, canonical path is the identity anchor.

**Rationale**:

- Canonical paths are deterministic and collision-free within a machine.
- Works with the current spore workspace discovery model (which also uses paths).
- Avoids the need for a separate workspace registry before it exists.
- Symlinks and relative paths are normalized automatically, preventing the same workspace from being treated as multiple workspaces due to path differences.
- Allows for straightforward file-system-based session guards: a workspace's lock file or active session marker lives at a canonical location.
- Migration path is clear: once spore workspace records are stable and machine-canonical, a future design revision can add a stable ID layer on top, keyed by canonical path.

## Consequences

**Step 2** (add workspace context to volva session lifecycle) will:
- Add a `workspace_id` field to the session record, resolved from the canonical workspace root path at session start.
- Implement a cardinality guard in `volva-runtime` that rejects or queues a session start when the workspace already has an active session (unless override is configured).
- Extend the host envelope to include workspace identity so downstream tools (canopy, hyphae, cap) know which workspace a session is in.
- Update `volva backend doctor` to report active session count per workspace.

**Step 3** (wire workspace routing into canopy) will:
- Extend canopy's task dispatch to check workspace affinity when assigning agents.
- Allow agents to declare their current workspace via registration or heartbeat update.
- Log workspace mismatches clearly without hard rejection initially, allowing gradual adoption.

**Step 4** (add septa contract for workspace-session events) will:
- Define `workspace_session_started`, `workspace_session_ended`, and `workspace_session_conflict` event schemas.
- Emit those events from volva so cap, hyphae, and canopy can observe workspace-session patterns without polling.

## Non-Goals

This design intentionally defers:

- **UUID-based stable workspace identity**: requires stable spore workspace records, which are still being stabilized.
- **Distributed workspace coordination**: assumes single-machine local-first operation; multi-machine workspace state is future work.
- **Automatic workspace switching within a session**: sessions are workspace-scoped; workspace changes are explicit handoffs.
- **Workspace-scoped memory isolation**: hyphae stores memories keyed by project/workspace, but that is orthogonal to volva's session binding and is owned by hyphae.
- **Priority or queueing for blocked session starts**: a workspace occupied by an active session rejects new sessions outright; queueing is a future enhancement if canopy needs it.

## References

- `volva-core`: `WorkspaceBinding`, `ExecutionSessionIdentity`, `ExecutionSessionState`
- `volva-runtime`: session lifecycle, persistence, and hook emission
- Canopy architecture: current single-workspace agent dispatch model; future workspace-affinity routing
- Septa: cross-tool contract schemas for session and workspace events
