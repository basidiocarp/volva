# Volva Architecture

Volva is the execution-host layer for the Basidiocarp ecosystem. It owns backend routing, host-context assembly, hook dispatch, and the thin operator surface around those responsibilities. It does not own memory, code intelligence, lifecycle capture, or install policy; those stay in sibling repos.

The current host model is session-first. `volva-core` owns the typed session and workspace identity pieces:

- `ExecutionSessionId`
- `WorkspaceBinding`
- `ExecutionParticipantIdentity`
- `ExecutionSessionIdentity`
- `ExecutionMode`
- `ExecutionSessionState`

`volva-cli` builds those identities for `run` and `chat`. `volva-runtime` threads the same identity through request validation, prompt assembly, hook payloads, and a persisted host-session snapshot under Volva's vendor directory. `backend session` reads that last persisted snapshot instead of inventing a new session id on inspection. That keeps the host boundary explicit instead of rebuilding session state in multiple places.

## Current Flow

1. `volva-cli` loads config and resolves the current workspace root.
2. The CLI constructs an execution session identity for the chosen mode.
3. `volva-runtime` persists the current host-session snapshot and validates the backend.
4. The selected backend runs with the prepared prompt.
5. Hook adapters receive normalized session events.
6. `backend session` exposes the latest persisted session snapshot without launching a run.

## Host Session Surface

The host-session surface is intentionally small. It exists so operators can inspect the latest persisted host session that Volva observed during `run` or `chat`.

It reports:

- backend kind
- backend command
- whether `run` is supported
- session id
- mode
- workspace root
- worktree id
- primary participant
- session state

The current implementation persists `active` and `finished` state for normal runs, and `paused` / `resumed` during Anthropic API retry windows.

## Repository Boundaries

- `volva-cli`: command entrypoints and operator UX.
- `volva-runtime`: backend invocation, prompt assembly, hook routing, and session inspection.
- `volva-core`: shared session, workspace, and status types.
- `volva-api`: Anthropic API request path.
- `volva-auth`: auth and credential handling.
- `volva-config`: config loading and backend selection.
- `volva-adapters`, `volva-bridge`, `volva-compat`, `volva-tools`: support crates.

## Compatibility Note

This file is canonical. `docs/architecture.md` remains as a compatibility alias for older links and should not be treated as the source of truth.
