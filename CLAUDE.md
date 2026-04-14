# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Volva is the execution-host layer for the Basidiocarp ecosystem. It is a 10-crate Rust workspace centered on `volva-cli`, `volva-runtime`, `volva-auth`, `volva-config`, `volva-api`, and `volva-core`; the remaining crates are support layers. Volva owns backend selection, host context assembly, auth handoff, and hook routing. It defers memory, code intelligence, coordination, and install policy to sibling repos.

## Crate Status

Active implementation crates:

- `volva-cli`
- `volva-runtime`
- `volva-auth`
- `volva-config`
- `volva-api`
- `volva-core` as the shared foundation for enums, auth/status types, and shared constants

Thin support or stub crates:

- `volva-adapters`
- `volva-bridge`
- `volva-compat`
- `volva-tools`

Keep that distinction explicit when updating docs or planning work. The support crates are intentionally small and mostly placeholder-like today.

---

## Operating Model

- Do not replace Hyphae, Rhizome, Canopy, Cortina, or Stipe.
- Do not let the CLI absorb backend internals. Orchestration stays thin.
- Do not spread runtime state beyond `./volva.json`, `./vendor`, and `~/.volva/auth/anthropic.json`.
- Do not treat the support crates as fuller products than they are today.
- Do not document a hook surface larger than the runtime actually emits.

## What Volva Does NOT Do

- Does not orchestrate workflows or manage dispatch decisions (Hymenium owns that)
- Does not provide operator utilities like statuslines (Annulus owns that)

---

## Failure Modes

- **Backend command missing**: `volva run` fails before launch and `backend doctor` reports the command as unresolved.
- **Auth missing or expired**: `volva chat` fails until `ANTHROPIC_API_KEY` is set or `volva auth login anthropic` completes.
- **Hook adapter misconfigured**: the runtime warns and keeps going, but hook delivery is degraded or absent.
- **Callback server failure**: OAuth login cannot complete because the local callback listener never receives the authorization code.
- **Config drift**: `volva.json` loads, but a wrong backend command, vendor path, or hook adapter path breaks runtime behavior.

---

## State Locations

| What | Path |
|------|------|
| Workspace config | `./volva.json` in the current working directory |
| Vendor directory | `./vendor` by default, resolved relative to the current working directory |
| Saved Anthropic auth | `~/.volva/auth/anthropic.json` |
| Runtime logs | stderr |

---

## Build & Test Commands

```bash
cargo build --release
cargo test
cargo clippy
cargo fmt

cargo test -p volva-runtime
cargo test -p volva-auth

cargo run -p volva-cli -- doctor
cargo run -p volva-cli -- backend status
cargo run -p volva-cli -- backend doctor
cargo run -p volva-cli -- auth status
cargo run -p volva-cli -- chat "say hello"
cargo run -p volva-cli -- run "summarize the repository"
```

---

## Architecture

```text
volva-cli
├── volva-runtime ──► volva-adapters
│                 ├──► volva-auth ──► volva-core
│                 ├──► volva-bridge
│                 ├──► volva-config ─► volva-core
│                 ├──► volva-tools ──► volva-core
│                 └──► volva-core
├── volva-api ───────► volva-core
├── volva-auth ──────► volva-core
├── volva-compat
├── volva-config ────► volva-core
└── volva-core
```

- **volva-cli**: six top-level commands, plus auth and backend subcommands, for operator-facing flows.
- **volva-runtime**: assembles the host envelope, runs the selected backend, and emits hook events.
- **volva-auth**: Anthropic OAuth PKCE flow, callback handling, token storage, and credential resolution.
- **volva-api**: direct Anthropic messages API path used by `volva chat`.
- **volva-config**: loads `volva.json`, default backend settings, and hook-adapter config.
- **volva-core**: shared enums and status types for backend, auth, and runtime reporting.
- **volva-adapters**, **volva-bridge**, **volva-compat**, and **volva-tools**: thin support crates today, not the main implementation path.

---

## Hook Adapter Contract

The runtime sends hook events to external adapters as JSON over stdin. The child process runs with `current_dir` set to the request cwd, and the timeout comes from `hook_adapter.timeout_ms`, which defaults to `30000`.

Current emitted phases:

- `session_start`
- `before_prompt_send`
- `response_complete`
- `backend_failed`
- `session_end`

Current payload shape:

```json
{
  "schema_version": "1.0",
  "phase": "before_prompt_send",
  "backend_kind": "official-cli",
  "cwd": "/path/to/project",
  "prompt_text": "[volva-host-context]\n...",
  "prompt_summary": "summarize the repo",
  "stdout": "optional backend stdout",
  "stderr": "optional backend stderr",
  "exit_code": 0,
  "error": "optional launch or runtime error"
}
```

The supported external adapter path today is the Cortina hook-event surface, typically configured as `cortina adapter volva hook-event`.

### Contract Validation

When changing the hook payload shape, validate the fixture against the canonical schema:

```bash
cd ../septa && bash validate-all.sh
```

The relevant schema is `volva-hook-event-v1`. Update the septa schema and fixture first, then change the runtime code.

---

## Auth Flow

`volva auth login anthropic` defaults to the Claude.ai target. `--console` switches the target to the Anthropic console flow.

The current flow is:

1. Generate PKCE verifier, challenge, and state.
2. Start a local callback server and optionally open the browser.
3. Exchange the returned code for OAuth tokens.
4. For the console target, optionally mint an API key from the OAuth session.
5. Save the resulting credential state to `~/.volva/auth/anthropic.json`.

`volva chat` prefers `ANTHROPIC_API_KEY` when it is set. Otherwise it falls back to saved credentials from `volva-auth`.

---

## Known Gaps

- `volva run` only supports the `official-cli` backend today. `anthropic-api` is intentionally routed through `volva chat`.
- The hook runtime emits five phases, which is smaller than the broader event set described in `docs/VOLVA-ARCHITECTURE.md`.
- `volva-adapters`, `volva-bridge`, `volva-compat`, and `volva-tools` are still thin crates.
- The zero-test crates are `volva-adapters`, `volva-bridge`, `volva-compat`, `volva-core`, and `volva-tools`.

---

## Testing Strategy

- Most current coverage sits in `volva-cli`, `volva-runtime`, `volva-auth`, `volva-api`, and `volva-config`.
- Backend work should be checked through both `backend doctor` and the runtime code path that actually launches the backend.
- Auth changes should be validated against both saved-token and environment-key paths.
- Hook changes should be tested against the real JSON adapter payload, not only against the higher-level architecture notes.
