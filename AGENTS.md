# Volva Agent Notes

## Purpose

Volva work produces host and runtime changes for the execution layer. Keep the CLI thin, keep backend behavior in the runtime and auth crates, and keep cross-tool payload changes synchronized with `../septa/`.

---

## Source of Truth

- `crates/` is source. Treat the crate code as authoritative.
- `docs/` is the repo-level design record. Keep it in sync with behavior changes.
- `../septa/` owns cross-tool payload schemas and fixtures. Update those first when a boundary changes.
- `target/` is generated output. Do not hand-edit it.

When schema and implementation drift, update the schema and fixture first, then the code that emits or consumes the payload.

---

## Before You Start

Before writing code, verify:

1. **Contracts**: Read `../septa/README.md` and the relevant schema or fixture before changing hook payloads.
2. **Versions**: Check `../ecosystem-versions.toml` before changing shared dependencies.
3. **Seams**: Keep backend, auth, config, and runtime responsibilities in their owning crates.
4. **Cross-tool impact**: If a type or payload crosses repos, update Volva and Septa in the same change.

---

## Preferred Commands

Use these for most work:

```bash
cargo build --release && cargo test
```

For targeted work:

```bash
cargo test -p volva-runtime
cargo test -p volva-auth
cargo run -p volva-cli -- backend status
cargo run -p volva-cli -- backend doctor
```

---

## Repo Architecture

Volva is trying to stay a thin host shell with explicit seams. The CLI should orchestrate; it should not grow backend internals or auth state.

Key boundaries:

- `crates/volva-cli` orchestrates commands and operator-facing flows.
- `crates/volva-runtime` owns backend execution, context shaping, and hook emission.
- `crates/volva-auth` owns OAuth and credential storage.
- `crates/volva-config` owns config loading and backend selection.
- `crates/volva-core` stays shared types and status enums only.
- `crates/volva-adapters`, `crates/volva-bridge`, `crates/volva-compat`, and `crates/volva-tools` stay support layers until they justify more weight.

Current direction:

- Keep host policy explicit at the runtime seam.
- Keep provider-specific code behind the backend and auth crates.
- Keep cross-tool payloads versioned in Septa rather than embedded ad hoc in code.

---

## Working Rules

- Do not move backend logic into the CLI.
- Do not change hook payloads without updating the matching Septa schema and fixture.
- Prefer extending shared runtime helpers over adding backend-specific branches in multiple crates.
- Do not hand-edit generated output or vendor artifacts.
- Run the narrowest relevant `cargo test` plus `cargo fmt` or `cargo clippy` when behavior changes.

---

## Multi-Agent Patterns

For substantial runtime or contract work, use at least two agents:

**1. Primary implementation worker**
- Owns the write scope for the feature or refactor
- Specific files in scope: the touched crate(s), matching docs, and any affected Septa schema or fixture
- Does not cross into unrelated sibling repos

**2. Independent validator**
- Does not duplicate implementation. Reviews the broader shape.
- Specifically looks for:
  - host-model drift
  - auth-flow regressions
  - hook-contract drift
  - missing Septa updates

If the validator finds real structural issues, fix those before polishing output.

---

## Skills to Load

Default:

- `basidiocarp-workspace-router` - route cross-repo work to the right boundary
- `basidiocarp-rust-repos` - follow the repo-local Rust workflow
- `systematic-debugging` - explain failures before changing behavior

Situational:

- `test-writing` - when behavior changes need coverage
- `auth-implementation-patterns` - when changing auth or credential flow
- `writing-voice` - when updating prose docs
