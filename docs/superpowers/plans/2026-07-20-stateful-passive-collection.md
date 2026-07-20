# Stateful Passive Collection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace per-refresh full file rescans with a cached passive collector that discovers supported sessions automatically and tails only appended data.

**Architecture:** Keep the public runtime API stable. A new `LiveCollector` owns source discovery, per-file cursors, process lifecycle state, event deduplication, and the existing aggregator. `runtime_live.rs` re-exports the legacy ingest/replay surfaces while routing `load_snapshot` through a process-global collector cache.

**Tech Stack:** Rust 1.85, Tokio, Chrono, Serde JSON, existing llmeter adapters and aggregator; no new crate dependencies.

## Global Constraints

- Do not modify third-party CLI configuration during passive collection.
- Do not add network access to the default TUI path.
- Do not store prompt, response, thought, or tool payload content.
- Keep all existing CLI commands and function signatures source-compatible.
- Source discovery interval defaults to 2 seconds.
- Initial read per native source is capped at 4 MiB.
- Symlinks are never followed.

---

### Task 1: Tool-specific source catalog

**Files:**
- Create: `src/live/source_catalog.rs`
- Test: `src/live/source_catalog.rs` unit tests

**Interfaces:**
- Consumes: `registry::all_tools()`, `ToolDescriptor::resolve_session_roots()`
- Produces: `scan_session_sources(home, max_depth, max_per_tool, max_total) -> Vec<DiscoveredSource>`

- [ ] Write tests that create Codex rollout, Grok updates, Claude transcript, and unrelated config files under a temporary home.
- [ ] Verify tests fail because the catalog does not exist.
- [ ] Implement symlink-safe recursive discovery with tool-specific filename rules and newest-first limits.
- [ ] Verify the catalog tests pass.

### Task 2: Incremental line and source cursors

**Files:**
- Create: `src/live/cursor.rs`
- Test: `src/live/cursor.rs` unit tests

**Interfaces:**
- Produces: `SourceCursor::poll() -> CursorPoll`, `JournalCursor::poll() -> CursorPoll`
- Uses: `adapter_for`, `AdapterContext`, `TelemetryEvent`

- [ ] Write tests for unchanged reads, append-only reads, partial lines, truncation, and journal UUID replay.
- [ ] Verify tests fail before cursor implementation.
- [ ] Implement file identity, checkpoint, bootstrap cap, pending line, and adapter state retention.
- [ ] Verify cursor tests pass.

### Task 3: Stateful collector and correlation

**Files:**
- Create: `src/live.rs`
- Create: `src/live/correlation.rs`
- Test: `tests/live_collection.rs`

**Interfaces:**
- Produces: `LiveCollector`, `LiveCollectorConfig`, `load_cached_snapshot`
- Consumes: source catalog, source cursors, `detect_processes`, `Aggregator`

- [ ] Write tests proving a second refresh does not double-count unchanged source data.
- [ ] Write tests proving newly appended output is applied exactly once.
- [ ] Write tests for exact-PID, single-pair, and ambiguous process correlation.
- [ ] Implement source/process scan schedules, process lifecycle, bounded event UUID deduplication, and snapshot correlation.
- [ ] Verify live collection tests pass.

### Task 4: Runtime compatibility wrapper

**Files:**
- Create: `src/runtime_live.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Re-exports: `connect_sse`, `ingest_file`, `ingest_reader`, `snapshot_from_journal`, `wrap_command`, `IngestStats`
- Replaces: `runtime::load_snapshot`

- [ ] Add a compile-level integration test importing every existing runtime public surface.
- [ ] Route the `runtime` module to `runtime_live.rs`.
- [ ] Include `runtime.rs` as a private legacy module and re-export unchanged APIs.
- [ ] Delegate `load_snapshot` to `load_cached_snapshot`.
- [ ] Verify existing CLI/TUI tests compile and pass.

### Task 5: Documentation and full verification

**Files:**
- Modify: `README.md`
- Create: `docs/superpowers/specs/2026-07-20-stateful-passive-collection-design.md`
- Create: `docs/superpowers/plans/2026-07-20-stateful-passive-collection.md`

- [ ] Document that passive scanning is automatic and `setup` is optional enhanced telemetry.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `RUSTFLAGS=-Dwarnings cargo test --all-targets`.
- [ ] Run `cargo clippy --all-targets -- -D warnings`.
- [ ] Run `git diff --check`.
- [ ] Commit and update `master` without invoking GitHub Actions.
