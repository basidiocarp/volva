# Changelog

All notable changes to Volva are documented in this file.

## [Unreleased]

## [0.1.4] - 2026-04-14

### Added

- **Release automation**: GitHub release builds now publish cross-platform
  Volva artifacts so `stipe install volva` can download released binaries.

## [0.1.3] - 2026-04-09

### Changed

- **Release profile tuning**: Volva now carries explicit dev and release
  profile settings for smaller release artifacts and better debug info in local
  development builds.
- **Docs structure**: The docs set now includes a central `docs/README.md`
  with lowercase architecture, boundary-audit, hook-adapter, and backend-plan
  references.

## [0.1.2] - 2026-04-08

### Changed

- **Foundation alignment**: auth and CLI docs now describe Volva's runtime and
  host boundary more explicitly.

### Fixed

- **Auth tracing continuity**: correlation-aware spans now stay attached across
  Anthropic login, callback handling, and retry boundaries.
- **Retry diagnostics**: shared tracing now carries the failure-local detail
  operators need for auth retries and callback parsing issues.

## [0.1.1] - 2026-04-08

### Added

- **Auth tracing hardening**: Anthropic login now carries correlation-aware
  spans through session start, browser launch, callback wait, token exchange,
  and API-key minting boundaries.

### Fixed

- **Retry diagnostics**: Anthropic API retry and backoff messages now flow
  through the shared Spore tracing contract instead of raw stderr notices.
- **Callback locality**: OAuth callback parsing and retry behavior now emit
  failure-local tracing so invalid callback attempts are easier to diagnose.
- **Operator guidance**: Logging docs now explain the default lifecycle span
  behavior and the shared tracing coverage for auth and API flows.

## [0.1.0] - 2026-04-08

### Added

- **Shared logging rollout**: Volva now initializes logging through Spore's
  app-aware `VOLVA_LOG` path at the CLI boundary.
- **Runtime tracing**: Backend execution, hook adapter subprocesses, and
  Cortina status/doctor probes now emit shared tracing spans with workspace-
  aware context for faster failure localization.

### Fixed

- **Operator guidance**: Docs now distinguish debug logging from normal CLI
  stdout and runtime stderr surfaces.
