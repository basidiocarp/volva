# Volva Docs

These are the canonical design and planning docs for Volva. The tracked surface
is intentionally small; ignored `../.archive/docs/` and `../.handoffs/` hold
supporting material, not the current source of truth.

Start here:

- [architecture.md](architecture.md): execution-host ownership boundary,
  backend model, and repo layout
- [official-backend-plan.md](official-backend-plan.md): near-term plan for the
  first production backend
- [plans/README.md](plans/README.md): active planning entrypoint
- [hook-adapter-cortina.md](hook-adapter-cortina.md): supported Cortina adapter
  path and smoke-test flow
- [ecosystem-boundary-audit.md](ecosystem-boundary-audit.md): overlap analysis
  and repo-boundary notes

Then use the root docs for the operator view:

- [../README.md](../README.md): what Volva does and how to run it
- [../CLAUDE.md](../CLAUDE.md): implementation guidance and repo boundaries
- [../AGENTS.md](../AGENTS.md): agent workflow and cross-tool constraints
