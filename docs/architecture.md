# Volva Architecture

## Purpose

Define a new execution-host CLI and runtime layer for the Basidiocarp
ecosystem.

Working name: `volva`

The intent is not to replace the ecosystem. The intent is to own the execution-
host layer that Basidiocarp does not currently own:

- backend orchestration for supported agent runtimes
- headless execution-host CLI behavior
- context, memory, and token hygiene around agent runs
- local hook, MCP, and skill integration for the ecosystem
- native API-key backend support where direct API control is needed

Everything else should delegate back into the existing ecosystem where possible.

## Why `volva`

`volva` is the enclosing wrapper around the young fruiting body. That fits the
job of this tool:

- it wraps agent-runtime behavior
- it sits around existing Basidiocarp primitives
- it can present a backend-compatible CLI while still routing into local
  runtime systems you control

## Vendor Convention

Keep reference repositories under `volva/vendor/`, but keep them out of
version control.

`volva/.gitignore` should ignore everything under `vendor/` except
`vendor/.gitignore`.

Suggested local layout:

```text
volva/vendor/
  claude-code-main/
  claw-code-parity/
  claurst/
  codex-rs/
  claude-ts-reference/  # optional if kept separately from claude-code-main
```

Suggested clone commands:

```bash
git clone <local-or-private-claude-code-main> volva/vendor/claude-code-main
git clone <claw-code-parity-remote-or-local-clone> volva/vendor/claw-code-parity
git clone https://github.com/Kuberwastaken/claurst volva/vendor/claurst
git clone https://github.com/openai/codex.git volva/vendor/codex-rs
git clone <private-or-local-ts-reference> volva/vendor/claude-ts-reference
```

During the upstream `claw-code` migration, use the temporary
`claw-code-parity` mirror as the local harness reference.

Rules:

- treat `vendor/` repos as read-only reference material
- pin exact SHAs in design notes before borrowing behavior
- never build Basidiocarp features directly inside `vendor/`
- extract behavior and reimplement it inside `volva`

## Ownership Boundary

`volva` should own:

- backend orchestration and execution-host policy
- execution-host CLI entrypoint and session UX
- context assembly, compaction, and token hygiene
- hook execution, policy, and adapter routing
- MCP, skill, and local tool registration policy
- native API-key backend support for direct API mode
- host-specific compatibility shims for first-party Claude behavior

`volva` should not own:

- long-term memory
  - use `hyphae`
- semantic code intelligence
  - use `rhizome`
- lifecycle signal capture
  - use `cortina`
- multi-agent coordination
  - use `canopy`
- install, update, doctor, and host repair
  - use `stipe`
- reusable skills and agent packaging
  - use `lamella`
- shared path, host, and config primitives that should live in a shared crate
  - use or extend `spore`

## High-Level Model

```text
User
  -> volva CLI
    -> backend selector
    -> context + hook runtime
    -> supported backend
       - official Claude Code headless backend
       - native Anthropic API backend
       - future provider backends
    -> Basidiocarp adapters
         -> hyphae
         -> rhizome
         -> cortina
         -> canopy
         -> stipe
```

The key design rules are:

- `volva` should shape context before any backend sees it
- `volva` should keep durable memory outside backend internals
- `volva` should use the official Claude backend for subscription users
- `volva` should use the native API backend for direct API users

## Source Weighting

Use the external references with different weights:

1. `claude-code-main`
   - behavior source of truth for the first backend and headless CLI shape
2. `claurst`
   - secondary Rust reference for Claude-first auth behavior and bridge ideas
3. `codex-rs`
   - architecture reference for host/runtime/platform engineering
4. `claw-code`
   - secondary Rust harness reference where its factoring is simpler than the
     others

`volva` should be a synthesis, not a fork:

- first-backend behavior from `claude-code-main`
- official CLI execution-host structure from `claude-code-main`
- host/runtime/config/sandbox structure from `codex-rs`
- selective Claude-first auth and bridge shortcuts from `claurst`
- selective harness ideas from `claw-code`

## Backend Strategy

`volva` should support multiple execution backends under one host contract.

Supported backends:

- `official-cli`
  - primary backend for Claude subscription and team users
  - wraps `claude -p` or later the Agent SDK
  - inherits official auth and tool behavior
- `anthropic-api`
  - primary native backend for direct API users
  - uses Console API keys
  - gives `volva` direct request/runtime control

Experimental backend:

- `anthropic-native-oauth`
  - compile-gated and non-ship
  - useful for local research and parity validation
  - should not be the product's primary path

Why this split:

- official backend minimizes auth and compatibility drift
- native API backend preserves a direct-control path
- experimental native OAuth stays available for research without becoming the
  product center of gravity

## Multi-Provider Strategy

`volva` should be one CLI with multiple provider backends, not one provider
hard-coded into the host.

Recommended provider model:

- Anthropic
  - first shipping target
  - official CLI backend for subscription auth
  - native API-key backend for direct API use
- OpenAI / ChatGPT
  - second provider backend
  - OAuth PKCE and account handling informed by `codex-rs`

The key rule is:

- auth, API clients, and provider quirks stay provider-specific
- hooks, tools, approvals, profiles, adapters, and operator UX stay host-level

This matters because it lets `volva` run both model families while reusing the
same local runtime spine. It also means an OpenAI backend can inherit the same
hook system and local tool surface instead of forcing a second parity effort.

Design stance:

- `volva` is not fundamentally Claude-only
- `volva` is a general execution host
- Claude compatibility is the first implementation target

Near-term sequencing:

- ship Anthropic official-cli first
- add Anthropic native API-key mode second
- keep the provider boundary clean from day one
- add OpenAI / ChatGPT as an experimental second provider after the host/runtime
  path is stable

## Repo Layout

Current workspace layout:

```text
volva/
  Cargo.toml
  README.md
  docs/
  crates/
    volva-cli/
    volva-core/
    volva-auth/
    volva-api/
    volva-runtime/
    volva-tools/
    volva-bridge/
    volva-config/
    volva-adapters/
    volva-compat/
```

## Crate Split

### `volva-cli`

Owns:

- top-level CLI commands
- REPL or TUI entrypoint
- auth login/logout/status commands
- config inspection commands
- session resume and status commands
- backend selection and headless job launch

Should be thin. It should mostly wire user input to runtime crates and backend
selection.

Primary references:

- `codex-rs` CLI and operator ergonomics
- `claurst` auth-oriented command surface
- `claw-code` CLI rendering where it stays simpler than `codex-rs`

### `volva-core`

Owns:

- shared domain types
- session ids
- message types
- tool call and tool result types
- usage accounting types
- model aliases
- auth metadata types

This should stay dependency-light and boring.

Primary references:

- `codex-rs` shared protocol and runtime types
- `claw-code` runtime session and usage types
- `claurst` only where auth-specific metadata needs a Claude-first shape

### `volva-auth`

Owns:

- supported native auth flows
  - Anthropic Console API-key resolution
  - OpenAI / ChatGPT OAuth PKCE flow behind a separate provider module
- token storage
- token refresh
- auth-mode detection
- API-key resolution
- account metadata where supported

Experimental-only:

- Anthropic native OAuth PKCE modules kept behind a feature flag for research

Design rule:

- every provider gets its own auth module, token namespace, and account-model
  mapping
- do not mix Anthropic and OpenAI token semantics

This is the most important new crate.

Primary references:

- `codex-rs` for login UX and credential-store discipline
- `claurst` for research-only Anthropic native OAuth behavior

### `volva-runtime`

Owns:

- headless backend invocation
- context assembly and compaction
- hook dispatch and failure policy
- backend capability checks
- session lifecycle and retries

Design rule:

- this is where `volva` adds value even when the official Claude backend is
  doing the actual agent work

### `volva-api`

Owns:

- Anthropic HTTP client
- OpenAI / ChatGPT HTTP client
- SSE streaming parser
- retries and backoff
- model and endpoint selection
- request and response translation

Design rule:

- provider-specific request and stream handling normalize into one internal
  runtime event model

Primary references:

- `claude-code-main` request semantics and wire behavior
- `claw-code` provider factoring and SSE handling
- `codex-rs` transport and client-structuring ideas where they improve the
  Rust implementation

### `volva-runtime`

Owns:

- conversation loop
- tool selection and execution orchestration
- prompt assembly
- compaction hooks
- hook lifecycle, ordering, and event emission
- permission requests
- sandbox gating
- MCP invocation integration

Design rule:

- runtime behavior is provider-aware but provider-agnostic in structure
- provider backends advertise capability differences instead of forking the
  host runtime

Primary references:

- `codex-rs` runtime, config, sandbox, approval, and MCP boundaries
- `claw-code` runtime and hook concepts where they stay smaller or easier to
  port
- `claude-code-main` as the compatibility reference for prompt assembly,
  permissions, and tool-loop semantics

This crate should become the harness kernel.

### `volva-tools`

Owns:

- tool registry
- built-in tool implementations
- approval metadata
- tool schemas
- tool dispatch

The built-in tools should be the minimum needed for a Claude-compatible local
runtime. Everything ecosystem-specific should be adapter-backed where possible.

Primary references:

- `claude-code-main` for Claude-compatible tool expectations
- `codex-rs` for modular tool registration and server exposure shape
- `claw-code` for simpler local tool structure and isolation

### `volva-bridge`

Owns:

- optional `claude.ai` bridge session registration
- remote session polling
- event forwarding
- compatibility around remote-control or CCR-style browser-linked sessions

This crate should be optional and guarded as experimental because the endpoints
may drift.

Primary references:

- `claude-code-main` for bridge and remote-session behavior
- `claurst` bridge crate for the first useful Rust translation
- `codex-rs` for protocol formalization and machine-facing control surfaces

### `volva-config`

Owns:

- typed config loading
- precedence rules
- auth config
- runtime config
- hook config and per-profile hook policy
- MCP server config
- sandbox config
- tool-policy config

Primary references:

- `codex-rs` config/profile model and typed policy surface
- `claw-code` config loader where it stays lighter-weight than `codex-rs`

### `volva-adapters`

Owns Basidiocarp integration shims:

- `hyphae` adapter
- `rhizome` adapter
- `cortina` adapter
- `canopy` adapter
- `stipe` adapter

This crate is where `volva` becomes ecosystem-native without copying those
concerns internally.

### `volva-compat`

Owns:

- Claude-compatible file layout helpers
- Claude-compatible hook discovery and migration helpers
- import/export helpers for `~/.claude`
- bridge compatibility helpers
- config migration helpers
- feature probes for first-party behavior

Use this crate to quarantine drift-prone compatibility logic.

## Hook System

Hook parity matters.

Near term, `volva` should preserve the hook points and payload shape that
existing tooling expects. Long term, the ecosystem should make those hooks more
valuable by consuming them together, not by replacing them.

The boundary is:

- `volva` owns hook registration, ordering, execution, isolation, and payload
  normalization
- Basidiocarp components consume selected hook events for memory, lifecycle,
  code intelligence, coordination, packaging, install, and repair

### Required Hook Events

`volva` should expose at least these host-level events:

- `session-start`
- `before-prompt-send`
- `before-tool`
- `after-tool`
- `permission-requested`
- `permission-resolved`
- `response-complete`
- `session-end`
- `auth-changed`

Useful second-wave events:

- `compaction-start`
- `compaction-complete`
- `bridge-connected`
- `bridge-disconnected`
- `mcp-requested`
- `mcp-complete`

### Hook Payload Shape

Normalize events before handing them to hooks.

Every hook payload should carry a stable core envelope:

- session identity
- turn identity
- event name
- timestamp
- active profile
- cwd and workspace roots
- sandbox mode
- approval policy
- auth mode summary
- provider identity and selected model profile

Event-specific payloads should add:

- prompt metadata for `before-prompt-send`
- tool name, args summary, and approval metadata for `before-tool`
- tool result, duration, and failure details for `after-tool`
- requested scope and resolution details for permission events
- response usage, stop reason, and compaction markers for `response-complete`
- summary and diagnostics pointers for `session-end`

Hook payloads should prefer normalized references over raw blobs. Large tool
outputs, transcripts, or artifacts should be passed by path or handle when
possible.

### Routing Into The Ecosystem

The hooks should work locally even with no adapters enabled. When adapters are
present, route events like this:

- `cortina`
  - capture the full lifecycle stream
  - store tool outcomes, corrections, and session events
- `hyphae`
  - store promoted summaries, resolved diagnostics, and memory-worthy outcomes
  - avoid mirroring every raw event
- `rhizome`
  - serve code-aware tool work triggered from hooks or tools
  - do not use it as a generic event sink
- `canopy`
  - receive queue or delegation signals only when a hook explicitly triggers
    orchestration
- `lamella`
  - package reusable hook handlers, commands, skills, and compatibility assets
- `stipe`
  - install hook wiring
  - doctor broken hook paths or config
  - repair host-level integration drift

### Failure Policy

Hooks need explicit failure behavior. Do not make this implicit.

Recommended defaults:

- observe-only hooks fail open and log
- policy hooks can fail closed when explicitly configured
- every hook has a timeout budget
- hooks run in isolated subprocesses or equivalent isolation boundaries
- hook stderr is captured and attached to diagnostics

Profile and per-hook config should control:

- enabled or disabled state
- fail-open versus fail-closed mode
- timeout
- retry policy
- allowed environment passthrough
- event filtering

### Compatibility Strategy

Short term:

- preserve the hook events your current tools rely on
- preserve enough payload compatibility that those tools can keep working with
  minimal or no changes
- make the same hook contract available no matter which provider backend is
  active

Long term:

- keep the old hook-facing contract stable where it matters
- move persistence, memory extraction, orchestration, and repair into
  Basidiocarp systems behind adapters
- let tools target the normalized `volva` event schema instead of binding
  directly to each downstream system
- let Anthropic and OpenAI backends share the same hook and tool spine so new
  host features do not need to be implemented twice

The practical answer is that you still want hook parity now. The difference is
that `volva` should become the stable host spine, while the ecosystem provides
the heavier downstream behavior.

## Basidiocarp Integration Map

### `hyphae`

Use for:

- recall at session start
- storing session summaries
- storing resolved auth or runtime diagnostics if useful
- document and evidence retrieval

Do not recreate:

- memory extraction systems
- team-memory models
- local memory-file sprawl as a primary persistence system

### `rhizome`

Use for:

- definitions, references, structure
- safe symbol editing
- change impact
- diagnostics and rename

Do not recreate:

- local LSP orchestration unless it is strictly needed for Claude compatibility

### `cortina`

Use for:

- lifecycle event capture around `volva`
- tool outcome and correction signals
- session-end summaries

### `canopy`

Use for:

- task ownership
- multi-agent orchestration
- handoffs and queue state

Do not recreate:

- task systems inside `volva` beyond the minimum session-local affordances

### `stipe`

Use for:

- install
- auth doctor
- host setup
- repair flows

Long term, `stipe host setup volva` should install and wire the new tool.

## Reference Import Plan

### Take from `claude-code-main`

### Keep the behavior

- exact auth and callback behavior
- exact request/response wire semantics
- true command and tool contracts
- bridge and remote-session sequencing
- compaction behavior
- permission prompts
- Claude-specific MCP OAuth edge cases

### Do not copy directly

- monolithic file structure
- Bun-specific assumptions
- internal product couplings that do not help `volva`

### Take from `claurst`

### Keep the Rust shortcut

- Anthropic OAuth PKCE flow
- Claude.ai versus console auth split
- token refresh logic
- auth status surface
- selected bridge/session registration behavior
- selected settings-sync concepts only if still justified after Basidiocarp
  integration

### Do not copy directly

- memory systems that overlap `hyphae`
- task or coordination systems that overlap `canopy`
- large first-party-product mirror surfaces that do not materially improve the
  local runtime
- novelty or product dressing like `buddy`

### Take from `codex-rs`

### Keep the architecture

- workspace and crate boundary discipline
- profile-driven config model
- sandbox policy modeling
- shell escalation and approval plumbing
- app-server and protocol shape
- MCP client plus MCP server duality
- plugin and marketplace modeling where useful

### Do not copy directly

- OpenAI- or ChatGPT-specific auth behavior
- provider assumptions that conflict with Anthropic-first goals
- product surface area that over-owns Basidiocarp responsibilities

### Take from `claw-code`

### Keep the simpler harness ideas

- runtime boundaries
- config parsing style where it is lighter than `codex-rs`
- permissions and sandbox concepts
- hook runner concepts
- provider abstraction
- MCP runtime layout
- test discipline around runtime behavior

### Do not copy directly

- `claw.dev` product auth defaults
- product branding
- generic server model if it conflicts with Basidiocarp boundaries

## Migration Phases

### Phase 0: Reference Capture

Goal:

- pin exact SHAs for `claw-code`, `claurst`, `claude-code-main`, and
  `codex-rs`
- map `claude-code-main` source-of-truth modules
- write behavior notes before coding

Deliverables:

- reference matrix
- endpoint matrix
- auth flow notes
- bridge flow notes

### Phase 1: Auth Broker

Build first:

- `volva-auth`
- `volva-config`
- minimal `volva-cli auth login|logout|status`

Acceptance:

- Claude.ai login works
- token refresh works
- auth mode is explicit:
  - bearer subscription/team
 - console-generated API key
  - direct env API key
- team or subscription account state is detectable

### Phase 2: API and Runtime Shell

Build:

- `volva-api`
- `volva-core`
- `volva-runtime`
- minimal `volva-cli` prompt path

Acceptance:

- send a prompt
- stream responses
- execute a small built-in tool set locally before API continuation
- runtime config, sandbox mode, and approval policy can be selected explicitly

### Phase 3: Basidiocarp Adapters

Build:

- `volva-adapters`

Acceptance:

- session-start recall from `hyphae`
- semantic code operations through `rhizome`
- lifecycle capture through `cortina`

### Phase 4: Claude Compatibility

Build:

- `volva-compat`
- `volva-bridge`

Acceptance:

- selected Claude-compatible config and session flows work
- drift-prone features are clearly marked experimental

### Phase 5: Install and Operator Surfaces

Build:

- `stipe` integration
- `cap` visibility

Acceptance:

- `stipe host setup volva`
- `stipe doctor` reports auth and bridge health
- `cap` can display auth/session/runtime state where useful

## Initial Tool Surface

Start smaller than Claude Code itself.

Recommended built-ins:

- `Bash`
- `Read`
- `Write`
- `Edit`
- `Glob`
- `Grep`
- `WebFetch`
- `Task` only if it is thin and can later delegate into `canopy`

Recommended immediate integrations:

- `HyphaeRecall`
- `RhizomeDefinition`
- `RhizomeStructure`
- `RhizomeEdit`

This is enough to prove the value of local interception before the API call.

## Key Design Rules

- Keep auth isolated from runtime.
- Keep bridge code isolated from core runtime.
- Prefer adapters over duplicate subsystems.
- Use the TypeScript source for behavior fidelity, not architecture.
- Treat Anthropic first-party endpoints as volatile.
- Ship a narrow green slice first: auth, prompt, local tools, stream.

## Immediate Next Steps

1. Keep external references pinned under `volva/vendor/` and record SHAs in the
   audit docs before borrowing code or behavior.
2. Implement `volva-auth` first, using `claude-code-main` for behavior,
   `claurst` for the Rust shortcut, and `codex-rs` only for login UX and
   credential handling discipline.
3. Port `codex-rs`-style config, approval, and sandbox boundaries into
   `volva-config` and `volva-runtime`.
4. Add the minimum Claude-compatible tool surface and streaming prompt path.
5. Add Basidiocarp adapters before expanding bridge or broad Claude product
   parity.

## Bottom Line

`volva` should be the Claude-first runtime shell for Basidiocarp.

It should not become a replacement for `hyphae`, `rhizome`, `cortina`,
`canopy`, or `stipe`. It should become the missing host-runtime layer that
connects Anthropic auth and Claude-compatible behavior to those existing
ecosystem components.
