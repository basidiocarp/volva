# Changelog

All notable changes to Volva are documented in this file.

## [Unreleased]

### Changed

- **Changelog bootstrap**: Release headings and entry structure now follow the
  shared ecosystem changelog template.

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
