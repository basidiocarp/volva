# Volva

Execution-host and runtime layer for the Basidiocarp ecosystem. Owns backend
selection, context shaping, and runtime policy around supported agent backends.

Named after the fungal volva, the enclosing wrapper at the base of a young
fruiting body that surrounds and contains the emerging structure.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp).

---

## The Problem

The ecosystem has memory, code intelligence, hooks, and installation tooling,
but it still needs a host layer that sits directly in front of agent backends.
Without that layer, backend orchestration, context assembly, and runtime policy
stay fragmented or vendor-specific.

## The Solution

Volva is the execution host for supported backends. It sits in front of the
backend, shapes context before invocation, owns runtime policy at that seam,
and leaves memory, code intelligence, coordination, and install flows to the
rest of the ecosystem.

---

## The Ecosystem

| Tool | Purpose |
|------|---------|
| **[volva](https://github.com/basidiocarp/volva)** | Execution-host runtime layer |
| **[canopy](https://github.com/basidiocarp/canopy)** | Multi-agent coordination runtime |
| **[cortina](https://github.com/basidiocarp/cortina)** | Lifecycle signal capture and session attribution |
| **[hyphae](https://github.com/basidiocarp/hyphae)** | Persistent agent memory |
| **[lamella](https://github.com/basidiocarp/lamella)** | Skills, hooks, and plugins for coding agents |
| **[mycelium](https://github.com/basidiocarp/mycelium)** | Token-optimized command output |
| **[rhizome](https://github.com/basidiocarp/rhizome)** | Code intelligence via tree-sitter and LSP |
| **[spore](https://github.com/basidiocarp/spore)** | Shared transport and editor primitives |
| **[stipe](https://github.com/basidiocarp/stipe)** | Ecosystem installer and manager |

> **Boundary:** `volva` owns execution-host orchestration. `hyphae` owns
> memory, `rhizome` owns code intelligence, `cortina` owns lifecycle capture,
> `canopy` owns coordination, and `stipe` owns install and repair policy.

---

## Quick Start

```bash
cargo check
cargo run -p volva-cli -- backend status
cargo run -p volva-cli -- backend doctor
```

```bash
cargo run -p volva-cli -- auth status
cargo run -p volva-cli -- chat "say hello"
cargo run -p volva-cli -- paths
```

---

## How It Works

```text
User                  Volva                         Backend and ecosystem
────                  ─────                         ─────────────────────
choose backend  ─►    backend selector       ─►    official CLI or native API
send prompt     ─►    context assembly       ─►    runtime invocation
emit hook event  ─►   adapter routing        ─►    Cortina intake
need memory      ─►   ecosystem integration  ─►    Hyphae, Rhizome, Canopy
```

1. Select a backend: choose the runtime backend from config or CLI flags.
2. Assemble host context: shape cwd, model, and the host-owned prompt envelope before invocation.
3. Run the backend: execute the selected backend in headless or direct API mode.
4. Route host signals: forward normalized hook events into Cortina where applicable.
5. Delegate ecosystem services: keep durable memory, code intelligence, and coordination outside backend internals.

---

## Backend Strategy

| Backend | Role | Status |
|---------|------|--------|
| `official-cli` | Primary path for Claude subscription and team users | First shipping target |
| `anthropic-api` | Direct API-key backend | Secondary backend |
| Future providers | Additional host-compatible backends | Planned later |

---

## What Volva Owns

- Backend orchestration for supported runtimes
- Execution-host CLI behavior
- Context assembly and runtime policy
- Hook adapter routing at the host seam
- Provider-specific compatibility shims where the host needs them

## What Volva Does Not Own

- Long-term memory: handled by `hyphae`
- Semantic code intelligence: handled by `rhizome`
- Lifecycle signal classification: handled by `cortina`
- Multi-agent coordination: handled by `canopy`
- Installation and repair: handled by `stipe`

---

## Key Features

- Host-owned backend selection: keeps runtime choice explicit instead of baking it into one CLI.
- Context shaping: assembles host-level prompt and runtime metadata before backend execution.
- Hook adapter seam: can forward normalized events into Cortina.
- Provider boundary: keeps backend-specific auth and transport behind a common host contract.

---

## Architecture

```text
volva/
├── volva-cli      command surface
├── volva-runtime  backend execution and host context
├── volva-config   config loading and backend selection
├── volva-auth     auth support
├── volva-adapters host and backend adapters
├── volva-api      API-facing types
├── volva-tools    tool registration and policy helpers
└── docs/          architecture and implementation plans
```

---

## Documentation

- [docs/VOLVA-ARCHITECTURE.md](docs/VOLVA-ARCHITECTURE.md): architecture and ownership boundary
- [docs/IMPLEMENTATION-PLAN-OFFICIAL-BACKEND.md](docs/IMPLEMENTATION-PLAN-OFFICIAL-BACKEND.md): first backend plan
- [docs/HOOK-ADAPTER-CORTINA.md](docs/HOOK-ADAPTER-CORTINA.md): Cortina adapter path
- [docs/ECOSYSTEM-BOUNDARY-AUDIT.md](docs/ECOSYSTEM-BOUNDARY-AUDIT.md): ecosystem overlap and boundary notes

## Development

```bash
cargo check
cargo test
cargo clippy
cargo fmt
```

## License

See repository license.
