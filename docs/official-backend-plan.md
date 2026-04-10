# Volva Official Backend Plan

## Scope

Plan the first production-oriented backend slice for `volva` as an execution
host:

- `backend = "official-cli"`
- headless execution via `claude -p`
- pre/post hooks around headless runs
- context and memory shaping before invocation

This replaces native Claude OAuth as the primary path. Native Anthropic API-key
support remains a secondary direct API backend, and future backends should fit
the same execution-host contract.

## Goal

Use the official Claude Code backend as the first backend while making `volva`
the execution host for:

- hook parity
- context compaction
- memory routing through `hyphae`
- tool and MCP policy
- retries, recovery, and job orchestration

## Recommendation

Implement the official backend in two stages:

1. `claude -p` subprocess backend
2. optional Agent SDK backend later for structured events and richer control

Why:

- `claude -p` is the fastest path to a real supported backend
- it preserves official auth and tool behavior
- it gives `volva` a clean headless orchestration boundary
- the SDK can be added later if subprocess mode is too opaque

This is a first-backend plan, not a statement that `volva` should remain
Claude-only forever.

## Immediate Pivot Tasks

Do these before adding more features:

1. freeze native Claude OAuth as non-primary
2. make backend selection explicit in config and CLI
3. implement a thin `claude -p` subprocess runner
4. wrap that runner with hook phases and diagnostics
5. only then reintroduce memory hydration and compaction

For substantive multi-agent implementation work, create a handoff document in
`.handoffs/` before dispatching agents. The handoff should define the
write scope, constraints, acceptance criteria, and validation commands so the
worker and auditor are reviewing the same contract.

Practical freeze rules:

- do not expand native Claude OAuth surface right now
- keep native Anthropic API-key support compiling
- treat `volva chat` as the native API path until `volva run` exists
- do not promise tool-level hook parity on the subprocess backend yet

## First Working Slice

The first slice to build is:

```text
volva run --backend official-cli --prompt "summarize this repository"
```

It should:

- resolve the configured Claude binary path
- build a `claude -p` command line
- set cwd explicitly
- execute headlessly
- return stdout as the final answer
- return stderr and exit status as diagnostics when the run fails

Do not add memory hydration, hook adapters, or retries until this slice works.

## Backend Shape

Add backend selection to config:

```toml
[backend]
kind = "official-cli"
command = "claude"
```

Future examples:

```toml
[backend]
kind = "anthropic-api"
```

```toml
[backend]
kind = "official-sdk"
```

## What `volva` Owns In Official Mode

- command dispatch
- job ids and session ids
- cwd/profile resolution
- hook dispatch before and after runs
- memory hydration and compaction
- artifact capture and replay
- failure classification and retries

## What The Official Backend Owns

- subscription auth
- Claude's built-in tool loop
- permission semantics inside Claude
- Claude-native context management after invocation starts

Important distinction:

- `volva` does not need to rebuild Claude's built-in tools for the official
  backend
- custom Basidiocarp capabilities should be exposed through hooks, MCP, skills,
  and wrapper-level orchestration

## First CLI Shape

Add a narrow backend-aware command surface:

```text
volva run --prompt "..."
volva run --backend official-cli --prompt "..."
volva backend status
volva backend doctor
```

Keep `volva chat` as the native API path for now. Do not overload one command
with two unrelated transport semantics until the backend abstraction settles.
`volva backend doctor` is also the narrow operator-facing place to distinguish
local backend readiness from downstream hook health observed through supported
`cortina status` and `cortina doctor` surfaces for the current cwd.

## Crate Responsibilities

### `volva-cli`

Add:

- `run.rs`
- `backend.rs`

Responsibilities:

- parse backend-aware commands
- load config and select backend
- render final output and diagnostics
- keep `main.rs` as a thin dispatcher

### `volva-runtime`

Add:

- `backend/mod.rs`
- `backend/official_cli.rs`
- `context.rs`
- `hooks.rs`

Responsibilities:

- spawn `claude -p`
- pass cwd, prompt, and selected environment
- capture stdout/stderr/exit status
- run hook phases
- attach memory/context payloads before invocation

Implementation order inside this crate:

1. backend trait and result type
2. official CLI subprocess runner
3. hook shell around backend execution
4. context assembly seam with a small static host envelope prepended before
   backend launch
5. later context compaction
6. adapter routing into `hyphae` and `cortina`

### `volva-config`

Add:

- backend selection
- command path override
- default permission/tool policy for official runs

## Invocation Contract

Initial subprocess contract:

- command: `claude`
- mode: `-p` / `--print`
- pass cwd explicitly
- pass prompt as a single payload from `volva`
- capture stdout as final assistant text
- capture stderr as diagnostics

Do not attempt to parse undocumented internal Claude event streams in the first
slice.

Current implementation status:

- `volva-runtime` assembles a deterministic host-owned prompt envelope before
  calling `claude -p`
- the initial envelope is local-only and includes cwd, backend, and model when
  configured
- durable memory recall, compaction, and orchestration remain out of scope for
  this slice

## Hook Phases

Required phases in the first slice:

- `session-start`
- `before-prompt-send`
- `response-complete`
- `session-end`
- `backend-failed`

Optional later:

- `before-tool`
- `after-tool`
- `permission-requested`
- `permission-resolved`

Those later phases may require SDK mode or other structured surfaces. Do not
promise them on the subprocess backend until proven.

## Context Strategy

Before launching `claude -p`:

1. collect relevant session and project metadata
2. hydrate memory from `hyphae`
3. compact or summarize large artifacts
4. prepend `volva`-owned context envelope
5. launch the official backend

The point of `volva` in official mode is to improve what Claude starts with,
not to replace Claude once the run is in progress.

## Testing Plan

First verification targets:

- backend selection parses correctly
- `official-cli` command line is built correctly
- hooks fire in the expected order around a fake subprocess
- large prompt inputs are compacted before invocation
- failed subprocesses produce actionable diagnostics

Integration target later:

- run against a real local `claude -p` installation in an ignored/manual test
- the current `volva -> cortina` hook-adapter smoke path is documented in
  `docs/hook-adapter-cortina.md`

## Concrete Next Steps

Update the workspace in this order:

1. update `volva/README.md` to describe the backend pivot
2. add backend config types to `volva-config`
3. add `volva run` and `volva backend status` parsing to `volva-cli`
4. add a minimal backend abstraction to `volva-runtime`
5. implement `official_cli.rs` with subprocess execution only
6. add unit tests for command construction and failure handling
7. manually verify against a local `claude -p` install

## Exit Criteria

The first official backend slice is done when:

- `volva run --backend official-cli --prompt "..."`
  launches Claude headlessly
- `volva` wraps that run with session and hook events
- `volva` can hydrate memory before invocation
- failure output is better than calling `claude -p` directly

At that point, `volva` is already valuable even without native OAuth or native
tool injection.
