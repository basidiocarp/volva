# Volva

Execution-host and runtime layer for the Basidiocarp ecosystem. It owns backend selection, context shaping, and runtime policy at the host seam.

Named after the fungal volva, the protective wrapper around a young fruiting body.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp).

---

## The Problem

The ecosystem has memory, code intelligence, hooks, and installation tooling, but it still needs a host layer directly in front of agent backends. Without that layer, backend orchestration, context assembly, and runtime policy get scattered across tools and vendors.

## The Solution

Volva keeps the host boundary in one place. It chooses the backend, shapes the prompt and runtime context before invocation, and routes host signals to the right sibling systems. Memory, code intelligence, coordination, and install policy stay in the repos that own them.

---

## The Ecosystem

| Tool | Purpose |
|------|---------|
| **[volva](https://github.com/basidiocarp/volva)** | Execution-host runtime layer |
| **[mycelium](https://github.com/basidiocarp/mycelium)** | Token-optimized command output |
| **[hyphae](https://github.com/basidiocarp/hyphae)** | Persistent agent memory |
| **[rhizome](https://github.com/basidiocarp/rhizome)** | Code intelligence via tree-sitter and LSP |
| **[canopy](https://github.com/basidiocarp/canopy)** | Multi-agent coordination runtime |
| **[cortina](https://github.com/basidiocarp/cortina)** | Lifecycle signal capture and session attribution |
| **[lamella](https://github.com/basidiocarp/lamella)** | Skills, hooks, and plugins for coding agents |
| **[spore](https://github.com/basidiocarp/spore)** | Shared transport and editor primitives |
| **[stipe](https://github.com/basidiocarp/stipe)** | Ecosystem installer and manager |

> **Boundary:** `volva` owns execution-host orchestration. `hyphae` owns memory, `rhizome` owns code intelligence, `cortina` owns lifecycle capture, `canopy` owns coordination, and `stipe` owns install and repair policy.

---

## Quick Start

```bash
cargo check
cargo run -p volva-cli -- backend status
cargo run -p volva-cli -- backend doctor
cargo run -p volva-cli -- auth status
cargo run -p volva-cli -- chat "say hello"
```

---

## How It Works

```text
User                  Volva                         Backend and ecosystem
────                  ─────                         ─────────────────────
choose backend  ─►    backend selector       ─►    official CLI or native API
send prompt     ─►    context assembly       ─►    runtime invocation
emit hook event  ─►    adapter routing       ─►    Cortina intake
need memory     ─►    ecosystem integration  ─►    Hyphae, Rhizome, Canopy
```

1. Select a backend from config or CLI flags.
2. Assemble host context before invocation.
3. Run the chosen backend in headless or API mode.
4. Route normalized hook events into Cortina when applicable.
5. Keep durable memory, code intelligence, and coordination outside backend internals.

---

## What Volva Owns

- Backend orchestration for supported runtimes
- Execution-host CLI behavior
- Context assembly and runtime policy
- Hook adapter routing at the host seam
- Provider-specific compatibility shims where the host needs them

## What Volva Does Not Own

- Long-term memory, handled by `hyphae`
- Semantic code intelligence, handled by `rhizome`
- Lifecycle signal classification, handled by `cortina`
- Multi-agent coordination, handled by `canopy`
- Installation and repair, handled by `stipe`

---

## Key Features

- Host-owned backend selection keeps runtime choice explicit.
- Context shaping assembles host-level prompt and runtime metadata before backend execution.
- Hook adapter routing forwards normalized events into Cortina.
- Provider boundaries keep backend-specific auth and transport behind a common host contract.

---

## Architecture

```text
volva/
├── crates/volva-cli      command surface and orchestration
├── crates/volva-runtime  backend execution and host context
├── crates/volva-auth     OAuth, token storage, and credential resolution
├── crates/volva-config   config loading and backend selection
├── crates/volva-api      Anthropic API client path
├── crates/volva-core     shared types and status enums
├── crates/volva-adapters host and backend adapter glue
├── crates/volva-bridge   thin bridge crate
├── crates/volva-compat   compatibility shims
└── crates/volva-tools    tool registration and policy helpers
```

---

## Documentation

- [docs/README.md](docs/README.md) — docs index and reading order
- [docs/architecture.md](docs/architecture.md) — architecture and ownership boundary
- [docs/official-backend-plan.md](docs/official-backend-plan.md) — first backend plan
- [docs/hook-adapter-cortina.md](docs/hook-adapter-cortina.md) — Cortina adapter path
- [docs/ecosystem-boundary-audit.md](docs/ecosystem-boundary-audit.md) — ecosystem overlap and boundary notes

---

## Development

```bash
cargo build --release
cargo nextest run
cargo test
cargo clippy
cargo fmt
```

- Prefer `cargo nextest run` for the normal test loop.
- Keep `criterion` out of scope here until a concrete hot path is named.
- Use whole-command timing when backend or auth flows feel slow, for example
  `time cargo run -p volva-cli -- backend doctor`.
- Workspace profiles are tuned in `Cargo.toml`: release builds use LTO,
  single-codegen-unit, panic-abort, and strip; dev builds keep line tables so
  iteration stays usable without giving up too much debug context.
- reqwest stays the intended async HTTP stack for Volva's auth and API
  clients. This is a networked host layer, so that cost is expected.

## Logging

Volva writes diagnostic logs to stderr through Spore's shared logger so normal
CLI output stays readable.

- Use `VOLVA_LOG` for repo-specific logging, for example
  `VOLVA_LOG=volva=debug cargo run -p volva-cli -- backend doctor`.
- `RUST_LOG` still works as the broader Rust fallback, but `VOLVA_LOG` is the
  intended operator knob for this workspace.
- Default runtime logging is `warn`, with lifecycle span events enabled so
  auth, API, backend, and hook boundaries emit shared tracing without forcing
  `debug`.
- Logging is separate from normal product output: CLI responses still go to
  stdout, while backend stderr, hook diagnostics, and tracing stay on stderr.
- OAuth login, callback handling, and Anthropic API retry diagnostics now flow
  through the shared Spore tracing contract instead of ad hoc stderr notices.

## License

See repository license.
