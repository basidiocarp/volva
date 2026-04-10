# Basidiocarp — Ecosystem Overview & Gap Analysis

## The North Star

Make LLMs more efficient working on codebases — solo or in groups — without blowing up the context window.

Every tool in the ecosystem traces back to one of three problems:

1. **Too many tokens consumed** — agents read more than they need, commands dump more output than the model can use
2. **No memory between sessions** — every context compaction or restart starts from zero
3. **No coordination** — multiple agents doing parallel work have no shared ground truth about who owns what

---

## What the Ecosystem Is

Basidiocarp is a local-first harness that wraps AI coding agents. It doesn't replace the agent or the IDE; it sits between them and the codebase, shaping what the agent sees, remembering what happened, and coordinating when there's more than one agent in play.

The stack has three distinct layers:

**Layer 1 — Runtime efficiency** (reduce tokens in, improve quality of tokens in)
- `mycelium` — filters and compresses command output before it hits the context window
- `rhizome` — provides symbol-level code intelligence so agents navigate instead of read whole files

**Layer 2 — Memory and signal capture** (persist state across sessions)
- `hyphae` — episodic memories with decay + semantic knowledge graphs, hybrid search, training data export
- `cortina` — hook runner that intercepts lifecycle events and feeds structured signals into hyphae

**Layer 3 — Coordination** (multi-agent runtime state)
- `canopy` — agent registry, task ledger, structured handoffs, Council threads, evidence refs

**Supporting infrastructure**
- `spore` — shared Rust library: tool discovery, JSON-RPC, editor config, platform paths
- `stipe` — ecosystem installer and health manager
- `lamella` — skill/agent packaging for Claude Code (230 skills, 175 agents, 213 commands)
- `cap` — web dashboard: memory browser, token analytics, operator view

---

## Session Flow

At runtime, a single agent session looks like this:

```
Session start
  └─► hyphae recalls relevant memories and memoirs for current worktree

Agent works
  ├─► mycelium intercepts command output → filters/compresses → large outputs chunked to hyphae
  ├─► rhizome provides symbol navigation, references, structure on demand
  └─► cortina intercepts lifecycle hooks:
        PreToolUse  → optional tool redirect advisory
        PostToolUse → error detection, self-correction detection, build/test signals
        SessionEnd  → structured summary written to hyphae, rhizome export triggered

Post-session
  └─► cap can read memory health, token analytics, session outcomes
```

With multiple agents, canopy sits above this and tracks who owns what task, what's blocked, and what's waiting for review. Cortina events become canopy evidence refs; hyphae session IDs are attached to task decisions.

---

## Tool-by-Tool State

### mycelium — v0.x, active

CLI proxy that sits between the agent and the shell. `git log -20` returns 5 lines instead of 200. `cargo test` with 500 passing tests returns only the 2 failures. Small outputs pass through untouched, medium ones get filtered, large ones get chunked into hyphae. 70+ filters cover git, cargo, npm, docker, kubectl, and more.

**What's working:** Core filtering pipeline, large-output routing to hyphae, 60–90% token savings on common workflows, cross-platform path handling.

**Gaps:**
- No per-project analytics isolation; all savings are global totals
- No explain mode showing why a command was rewritten and what savings were achieved
- No cost tracking alongside token savings (`$X.XX` per session/project)
- Missing filters for `rg`, `bun`, `podman`, and AST-aware diffs

---

### hyphae — v0.x, active

Persistent memory for AI coding agents. Two complementary models: memories are episodic (decay over time based on importance and access frequency); memoirs are permanent knowledge graphs with typed relations between concepts. Search blends FTS5 full-text (30%) with cosine vector similarity (70%) using fastembed locally.

Decay formula: `effective_decay = base_rate * importance_multiplier / (1 + access_count * 0.1)`

Critical memories never decay. Frequently accessed ones slow down.

**What's working:** Dual memory model, hybrid search, session lifecycle tracking, structured feedback loop foundations (recall events, outcome signals, session-linked scoring hooks), training data export pipeline, rhizome memoir integration.

**Gaps:**
- Recall effectiveness scoring not wired: memories get recalled but outcomes don't feed back into ranking, so hyphae is write-mostly rather than learning
- No auto-ingest watcher for live worktrees — stale unless cortina triggers an export or you run manually
- Multi-project namespacing needs tightening; semantic deduplication and conflict detection between projects are incomplete
- No memory hygiene tools beyond basic commands (merge, pin, archive, trace, bulk cleanup)
- The cortina→hyphae signal bridge works for simple signals; richer structured outcome attribution is not yet flowing

---

### rhizome — v0.x, active

Code intelligence MCP server. Dual backend: tree-sitter (18 languages, 10 with dedicated query sets) for instant offline parsing, LSP (32 languages, 20+ auto-installed) for cross-file navigation. A `BackendSelector` picks the right backend per tool call. 37 MCP tools total covering symbol extraction, file editing, diagnostics, and code graph export.

Exports code structure graphs into hyphae memoirs so agents can query "what calls this function?" from memory rather than re-parsing the file.

**What's working:** Tree-sitter + LSP dual backend, auto-install of LSP servers, code graph export to hyphae, 37 MCP tools, cross-platform path and binary handling.

**Gaps:**
- No structural fallback for large or unsupported files: if tree-sitter and LSP both fail, the tool just fails rather than falling back to a parserless outline (indentation/bracket depth heuristics)
- No cross-file call graphs or dependency graphs beyond what individual LSP calls return
- No persistent workspace index or daemon for large repos; each request restarts parsing
- No refactor preview mode before applying rename/edit tools
- Rhizome-backed AST chunking in hyphae isn't wired: hyphae uses basic function boundaries for chunking rather than delegating to rhizome's tree-sitter parsers

---

### cortina — v0.2.5, active

Lifecycle signal runner. Reads host adapter envelope from stdin, normalizes into internal signal types, detects patterns in tool results, stores signals in hyphae. Adapter-first model: Claude Code adapter is the primary surface, but the shared signal pipeline is behind an explicit adapter boundary.

Also ships `cortina statusline` for Claude Code's statusline integration: outputs context window %, token counts, estimated session cost, model name, git branch, and mycelium savings.

**What gets captured:**

| Signal | Trigger | Stored as |
|--------|---------|-----------|
| Error | Bash exit != 0 | `errors/active` |
| Resolution | Same command succeeds after failure | `errors/resolved` + feedback signal |
| Self-correction | Edit after recent write to same file | `corrections` + feedback signal |
| Validation pass | Build or test succeeds | `build_passed` / `test_passed` signal |
| Test failure | Test runner with failures | `tests/failed` |
| Test fix | Test passes after failure | `tests/resolved` |
| Code changes | 5+ edits + successful build | Triggers `rhizome export` |
| Doc changes | 3+ doc edits | Triggers `hyphae ingest-file` |
| Session end | SessionEnd or Stop event | `hyphae session end` with structured summary |

**What's working:** Claude Code adapter covering PreToolUse, PostToolUse, SessionEnd; structured outcome ledger per worktree; worktree-scoped session isolation; statusline command; adapter-first architecture ready for new hosts.

**Gaps:**
- No Codex adapter: the adapter boundary exists, nothing is implemented
- `PreCompact` and `UserPromptSubmit` lifecycle events not captured — these are exactly the moments where context is about to be lost, and hyphae has no snapshot of what was happening right before compaction
- Cortina events are not flowing as canopy evidence refs; the cortina→canopy bridge is missing
- Richer structured outcome attribution is not implemented: cortina detects patterns (error after edit) but doesn't semantically attribute "this correction fixed this specific error from 3 tool calls ago"
- No configurable capture thresholds or dedupe policies exposed to operators yet

---

### canopy — v0.2.0, active

Coordination runtime. Task-scoped, local-first, one machine with multiple host adapters or worktrees. Not a generic chat room — a task ledger with explicit ownership semantics.

Uses its own SQLite store at `.canopy/canopy.db` rather than overloading hyphae.

**What's implemented:**
- Agent registry with `host_id`, `host_type`, `host_instance`, heartbeat + heartbeat history
- Task ledger with full lifecycle: creation, assignment, handoff, closure
- Task triage metadata: priority, severity, acknowledgment state, operator notes
- Task-event history: creation, assignment, transfer, status changes
- Handoff protocol with `due_at` / `expires_at` semantics and write-time validation
- Council message threads per task: `proposal`, `objection`, `evidence`, `decision`, `handoff`
- Evidence ref slots for: hyphae session IDs, cortina event IDs, rhizome impact-analysis results, mycelium output
- Snapshot presets and server-side triage filters
- Attention semantics: task/handoff/agent attention levels, freshness summaries, operator action hints
- Assignment history, reassignment counts, ownership summaries
- CLI surface with tests

**Gaps:**
- Cap integration not wired: the transport boundary (HTTP vs CLI-backed reads) isn't decided, so cap can't show canopy state
- Cortina→canopy evidence bridge missing: cortina events have IDs, canopy has slots for them, nothing connects them
- Capability routing not implemented: agent registry tracks host type but can't route tasks based on model capability
- Sub-task hierarchy not implemented: the `task_relationships` table supports parent/child but the attention model doesn't understand "all children complete = parent complete"
- Verification gates not implemented: tasks with `verification_required=true` can be marked complete without evidence
- `canopy import-handoff` (parse handoff markdown into task tree) not yet built
- File-scope conflict detection not implemented: no mechanism to detect two agents targeting overlapping file scopes

---

### cap — v0.x, active

Web dashboard. React + Mantine + Hono. Reads directly from hyphae SQLite and rhizome MCP.

**What's working:** Memory browser, token savings analytics, resolved config path provenance (shows which path was chosen and why), multi-host status and onboarding, platform-specific repair guidance.

**Gaps:**
- No canopy integration: can't show active agents, task ownership, blocked tasks, pending handoffs, council summaries
- No session timeline: no joined view of recalls, tool errors, fixes, tests, exports, and summaries in chronological order
- No lessons view: no surface showing which memories actually changed outcomes
- Per-project analytics vs global totals: everything is aggregated, no project-scoped breakdown
- No "copy fix command" or "run via stipe" actions from read-only surfaces; visibility stops short of repair

---

### lamella — v0.x, active

Plugin system for Claude Code. 230 skills, 175 agents, 213 commands across 20 plugins. Python-based build system. Exports to Claude Code plugin format and Codex skill folders.

**What's working:** Skill library, agent library, plugin build system, Claude Code plugin format output.

**Gaps:**
- Stale hook path bug ([#1](https://github.com/basidiocarp/lamella/issues/1)): no post-install validation detects when hardcoded paths in `~/.claude/settings.json` don't match the installed plugin location
- No plugin dependency resolution during install
- No `lamella` CLI wrapper for build, install, list, update
- Codex export parity with Claude output not complete

---

### stipe — v0.x, active

Ecosystem installer and manager. Multi-host, platform-aware. Handles install, init, doctor, update across macOS, Linux, and Windows.

**What's working:** Multi-host `host list`, `host setup`, `host doctor` flows; shared platform-aware install paths; convergence with spore for editor detection and MCP registration; health checks.

**Gaps:**
- No config drift detection for MCP servers, hooks, and generated config files
- No `--dry-run`, rollback, or richer repair flows
- No install profiles (`minimal`, `claude-code`, `cursor`, `full-stack`)
- No scheduled health checks or auto-repair
- No machine bootstrap import/export for moving a full setup between devices

---

### spore — library, stable

Shared Rust library consumed by all other tools. Tool discovery, JSON-RPC 2.0, subprocess MCP communication, editor detection, MCP config registration paths, platform-aware paths. Not user-facing.

**Status:** Stable and well-bounded. Low gap surface.

---

## Gap Analysis — Prioritized

### Tier 1: Broken feedback loop (fix these first)

**1. Cortina → Canopy evidence bridge**
Cortina captures lifecycle signals and writes them to hyphae. Canopy has typed evidence ref slots for cortina event IDs. Nothing connects them. The multi-agent operator view in canopy is blind to what actually happened during agent runs because evidence refs are empty. This is the most critical missing connection in the ecosystem.

**2. Recall effectiveness scoring**
Hyphae stores memories, cortina captures outcomes, but nothing closes the loop: "memory X was recalled before session Y, session Y resolved an error of type Z, therefore memory X gets a recall effectiveness bump." The `FEEDBACK-LOOP-DESIGN.md` exists, the session-linked recall hooks exist, but scoring isn't feeding back into retrieval ranking. Hyphae is write-mostly until this lands.

**3. Cap → Canopy operator views**
The transport boundary between canopy and cap isn't decided (HTTP surface vs CLI-backed reads). Until that's resolved and wired, canopy has no human-readable operator view. For multi-agent work this means the operator is flying blind.

### Tier 2: Missing features with clear owners

**4. Cortina PreCompact / UserPromptSubmit capture**
These are the lifecycle events fired right before context is about to be lost. Cortina currently captures signals *after* things happen but misses the pre-compaction window entirely. Adding these two event handlers would let hyphae snapshot the agent's current working state before the context window collapses.

**5. Codex adapter in cortina**
The adapter boundary was designed specifically for this. Claude Code is the only implemented adapter. Without Codex support, canopy can't meaningfully coordinate a Claude Code agent and a Codex agent on the same task because one side is dark.

**6. Rhizome structural fallback for large/unsupported files**
When tree-sitter and LSP both fail (unsupported language, file too large), rhizome returns an error and the agent falls back to reading the raw file. A parserless outline mode using indentation/bracket depth/entropy heuristics would give rhizome a graceful degradation path. This is a new backend, not a rewrite of the existing selector.

**7. Hyphae recall effectiveness → retrieval ranking**
Distinct from item #2 above (which is about closing the signal loop). This is about plumbing: once effectiveness scores exist, they need to actually weight retrieval results. The hybrid search currently does 30% BM25 / 70% cosine. Effectiveness scoring would add a third dimension.

**8. Rhizome-backed AST chunking in hyphae**
Hyphae uses basic function boundary detection for chunking. Rhizome has tree-sitter parsers for 18 languages that already do this correctly. Hyphae should delegate chunk boundary detection to rhizome rather than maintaining its own inferior version. This improves RAG quality across all 18 languages with no new parsing logic.

### Tier 3: Quality and polish (real, lower urgency)

**9. Lamella stale hook path bug (#1)**
No validation detects when hardcoded lamella paths in `~/.claude/settings.json` stop matching the installed plugin location after a version update. Silent breakage on every plugin update.

**10. Per-project analytics in mycelium and cap**
Everything is global totals. No way to ask "how much did this project's tests cost in tokens this week?" or compare token burn across projects.

**11. Canopy capability routing**
Agent registry tracks `host_type` and `host_instance` but has no logic for routing tasks based on model capability (Opus for planning, Sonnet for implementation, Haiku for validation). The multi-model orchestration hierarchy design exists but isn't implemented.

**12. No lifecycle integration test**
`cortina → hyphae → canopy → cap` has no end-to-end test. A `test-lifecycle.sh` should verify: session start → command execution → error capture → fix → resolution signal → session end → cap reads the timeline. Breaks in cross-tool integration are currently invisible until something is noticeably wrong.

**13. Mycelium explain mode**
No way to see why a command was rewritten or what filter matched. Useful for debugging unexpected compression behavior and for understanding what savings are coming from.

**14. Canopy sub-task hierarchy and verification gates**
The `task_relationships` table exists but the attention model doesn't understand parent/child completion semantics. Tasks with `verification_required=true` can be marked complete without evidence. Both are needed before canopy can enforce real verification workflows.

---

## What "Closed Loop" Looks Like

The ecosystem's full signal chain, when all the above gaps are closed:

```
Agent runs
  └─► cortina intercepts every hook
        PostToolUse → error/correction/build signals → hyphae memories
        SessionEnd  → structured summary → hyphae session + cortina event ID
                    → rhizome export triggered

cortina event ID → canopy evidence ref on active task
hyphae session ID → canopy evidence ref on active task

Next session
  └─► hyphae recalls memories with effectiveness weighting
        (memories that resolved similar errors rank higher)

Operator
  └─► cap shows:
        canopy task board (active agents, blocked tasks, pending handoffs)
        session timeline (recalls + errors + fixes in order)
        lessons view (which memories changed outcomes)
```

That's the complete harness. Right now the ecosystem has most of the pieces; the gaps are mostly in the connections between them rather than in the pieces themselves.
