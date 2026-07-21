# Live Session Correlation Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render exactly one TUI row per live user-facing LLM process, with current Codex/Grok native metrics when a matching log exists and no historical rows.

**Architecture:** Keep process discovery as the liveness authority. Expand Codex source discovery, timestamp process placeholders at their real start time, correlate PID-less native sessions by UUIDv7/start-time proximity, then reuse the existing PID-backed live filter.

**Tech Stack:** Rust 1.85, Tokio, Chrono, UUID, existing unit/integration tests only.

## Global Constraints

- Never display a native session without a currently live `(tool, pid)` pair.
- Keep an unmatched live process as a `NEW` placeholder.
- Add no dependency, process-tree inspection, or `lsof` call.
- Do not change replay mode or adapter wire formats.
- Use TDD: verify each new regression test fails before production edits.

---

### Task 1: Discover Orca Codex logs and exclude its daemon

**Files:**
- Modify: `tests/registry.rs`
- Modify: `src/registry.rs`

**Interfaces:**
- Consumes: `ToolDescriptor::resolve_session_roots(&Path) -> Vec<PathBuf>` and `parse_ps_line(&str) -> Option<ProcessInfo>`.
- Produces: Codex roots for default, Orca, `CODEX_HOME`, and `ORCA_CODEX_HOME`; process matching that rejects `codex app-server`.

- [ ] **Step 1: Write failing registry tests**

Add separate tests asserting:

```rust
#[test]
fn codex_session_roots_include_orca_runtime_home() {
    let codex = all_tools().iter().find(|tool| tool.id == ToolId::Codex).unwrap();
    let roots = codex.resolve_session_roots(Path::new("/home/example"));
    assert!(roots.iter().any(|path| path == Path::new(
        "/home/example/Library/Application Support/orca/codex-runtime-home/home/sessions"
    )));
}

#[test]
fn codex_app_server_is_not_a_user_session() {
    assert_eq!(parse_ps_line(" 3421 1224 91 codex app-server").unwrap().tool, None);
    assert_eq!(
        parse_ps_line(" 41526 54717 12 codex --yolo").unwrap().tool,
        Some(ToolId::Codex),
    );
}
```

Add an environment-root assertion that temporarily sets `CODEX_HOME`, calls `resolve_session_roots`, checks `<value>/sessions`, and restores the previous value.

```rust
#[test]
fn codex_home_env_adds_session_root() {
    let previous = std::env::var_os("CODEX_HOME");
    std::env::set_var("CODEX_HOME", "/tmp/codex-test-home");
    let codex = all_tools().iter().find(|tool| tool.id == ToolId::Codex).unwrap();
    let roots = codex.resolve_session_roots(Path::new("/home/example"));
    match previous {
        Some(value) => std::env::set_var("CODEX_HOME", value),
        None => std::env::remove_var("CODEX_HOME"),
    }
    assert!(roots.iter().any(|path| path == Path::new("/tmp/codex-test-home/sessions")));
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test --test registry codex_ -- --nocapture`

Expected: Orca root and daemon-exclusion tests fail against current code.

- [ ] **Step 3: Implement the minimum registry changes**

Change root expansion to build a mutable vector, append unique environment roots for Codex, and use the existing descriptor list for the static Orca root:

```rust
pub fn resolve_session_roots(&self, home: &Path) -> Vec<PathBuf> {
    let mut roots = self
        .session_roots
        .iter()
        .map(|root| {
            root.strip_prefix("~/")
                .map_or_else(|| PathBuf::from(root), |suffix| home.join(suffix))
        })
        .collect::<Vec<_>>();
    if self.id == ToolId::Codex {
        for key in ["CODEX_HOME", "ORCA_CODEX_HOME"] {
            if let Some(value) = std::env::var_os(key) {
                let root = PathBuf::from(value).join("sessions");
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
    }
    roots
}
```

Reject only the exact executable/subcommand pair `codex app-server`; do not reject arguments that merely contain that text. Add the static root:

```rust
session_roots: &[
    "~/.codex/sessions",
    "~/Library/Application Support/orca/codex-runtime-home/home/sessions",
],
```

Use a token-pair check so unrelated arguments are unaffected:

```rust
fn command_has_subcommand(command: &str, executable: &str, subcommand: &str) -> bool {
    let tokens = command_tokens(command).collect::<Vec<_>>();
    tokens.windows(2).any(|window| {
        Path::new(window[0]).file_name().and_then(|name| name.to_str()) == Some(executable)
            && window[1] == subcommand
    })
}
```

- [ ] **Step 4: Verify GREEN**

Run: `cargo test --test registry -- --nocapture`

Expected: all registry integration tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/registry.rs tests/registry.rs
git commit -m "fix: discover live Orca Codex sessions"
```

### Task 2: Preserve the real process start time

**Files:**
- Modify: `src/live.rs`

**Interfaces:**
- Consumes: `process_started_epoch_secs(&ProcessInfo, &DateTime<Utc>) -> i64`.
- Produces: `SessionDiscovered` events whose `occurred_at` is process start and `observed_at` is scan time.

- [ ] **Step 1: Write the failing test**

Extend `process_reconciliation_emits_discovery_heartbeat_and_exit_once` after the first reconcile:

```rust
let discovered = first
    .iter()
    .find(|event| matches!(event.kind, EventKind::SessionDiscovered { .. }))
    .unwrap();
assert_eq!(discovered.occurred_at, now - chrono::Duration::seconds(10));
assert_eq!(discovered.observed_at, now);
```

- [ ] **Step 2: Verify RED**

Run: `cargo test --lib process_reconciliation_emits_discovery_heartbeat_and_exit_once -- --nocapture`

Expected: `occurred_at` is currently `now`, so the new assertion fails.

- [ ] **Step 3: Implement the minimum timestamp change**

Pass the already calculated `started_epoch_secs` into `process_discovered` and construct the event as:

```rust
let started_at = DateTime::from_timestamp(started_epoch_secs, 0).unwrap_or(*now);
TelemetryEvent::new(
    tool,
    session_id,
    started_at,
    Confidence::Derived,
    EventKind::SessionDiscovered {
        pid: Some(pid),
        cwd: None,
        model: None,
    },
)
.with_observed_at(*now)
```

- [ ] **Step 4: Verify GREEN**

Run: `cargo test --lib process_reconciliation_ -- --nocapture`

Expected: PID reuse, discovery, heartbeat, and exit tests all pass.

- [ ] **Step 5: Commit**

```bash
git add src/live.rs
git commit -m "fix: retain process start time for correlation"
```

### Task 3: Match native sessions to live processes by start time

**Files:**
- Modify: `src/live/correlation.rs`

**Interfaces:**
- Consumes: `SessionSnapshot.session_id`, `started_at`, `pid`, `tool`, and UUID timestamp support already provided by `uuid`.
- Produces: `correlate_process_sessions(AppSnapshot) -> AppSnapshot` with at most one native row merged into each process row.

- [ ] **Step 1: Write failing correlation tests**

Add a helper that can set `started_at`, then add a regression using a known UUIDv7 timestamp:

```rust
#[test]
fn start_time_match_selects_current_native_and_live_filter_removes_history() {
    let process_started = Utc.timestamp_opt(1_784_639_940, 0).unwrap();
    let mut process = session(ToolId::GrokBuild, "process-7", Some(7), SessionState::New);
    process.started_at = process_started;
    let mut current = session(
        ToolId::GrokBuild,
        "019f84d4-737f-7b53-859b-7b519e34f571",
        None,
        SessionState::Stream,
    );
    current.started_at = process_started + chrono::Duration::minutes(5);
    let mut historical = session(
        ToolId::GrokBuild,
        "019f8493-10cf-7a71-b744-12a33cdfcf17",
        None,
        SessionState::Idle,
    );
    historical.last_seen_at = process_started + chrono::Duration::minutes(10);

    let correlated = correlate_process_sessions(snapshot(vec![process, historical, current]));
    let filtered = retain_live_process_sessions(correlated, &HashSet::from([
        (ToolId::GrokBuild, 7),
    ]));

    assert_eq!(filtered.sessions.len(), 1);
    assert_eq!(
        filtered.sessions[0].session_id,
        "019f84d4-737f-7b53-859b-7b519e34f571",
    );
    assert_eq!(filtered.sessions[0].pid, Some(7));
}
```

Add another test with two process rows and two close UUIDv7 native rows, asserting both PIDs occur exactly once. Keep the conflicting-PID test unchanged.

```rust
#[test]
fn start_time_match_claims_each_process_and_native_once() {
    let first_at = Utc.timestamp_opt(1_784_636_520, 0).unwrap();
    let second_at = Utc.timestamp_opt(1_784_640_410, 0).unwrap();
    let mut first_process = session(ToolId::Codex, "process-1", Some(1), SessionState::New);
    first_process.started_at = first_at;
    let mut second_process = session(ToolId::Codex, "process-2", Some(2), SessionState::New);
    second_process.started_at = second_at;
    let first_native = session(
        ToolId::Codex,
        "019f84a0-3971-79c1-b66e-37cb95a0de9c",
        None,
        SessionState::Stream,
    );
    let second_native = session(
        ToolId::Codex,
        "019f84db-942d-7920-936a-092152f36f01",
        None,
        SessionState::Stream,
    );

    let result = correlate_process_sessions(snapshot(vec![
        first_process,
        second_process,
        first_native,
        second_native,
    ]));
    let pids = result.sessions.iter().filter_map(|session| session.pid).collect::<HashSet<_>>();

    assert_eq!(result.sessions.len(), 2);
    assert_eq!(pids, HashSet::from([1, 2]));
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test --lib live::correlation::tests::start_time_ -- --nocapture`

Expected: the current 1-process/2-native fallback does not merge, so only the empty process row survives.

- [ ] **Step 3: Implement minimal deterministic matching**

After exact PID matching, build candidates only from live process-only rows and unclaimed, non-exited native rows with no PID. Use UUIDv7 time when available:

```rust
const START_MATCH_TOLERANCE_SECS: u64 = 60;

fn session_start_epoch_secs(session: &SessionSnapshot) -> i64 {
    Uuid::parse_str(&session.session_id)
        .ok()
        .and_then(|id| id.get_timestamp())
        .and_then(|timestamp| i64::try_from(timestamp.to_unix().0).ok())
        .unwrap_or_else(|| session.started_at.timestamp())
}
```

Create `(distance, process_index, native_index)` candidates for the same tool within 60 seconds, sort the tuples, then greedily claim each process and native index once. Call the existing `merge_process_into_native` for every claimed pair and remove the process placeholder. Delete the old `processes.len() == 1 && natives.len() == 1` fallback.

```rust
let mut candidates = Vec::new();
for process_index in process_indices(&snapshot.sessions) {
    if removed.contains(&process_index) {
        continue;
    }
    for (native_index, native) in snapshot.sessions.iter().enumerate() {
        if removed.contains(&native_index)
            || is_process_only(native)
            || native.state == SessionState::Exited
            || native.pid.is_some()
            || native.tool != snapshot.sessions[process_index].tool
        {
            continue;
        }
        let distance = session_start_epoch_secs(&snapshot.sessions[process_index])
            .abs_diff(session_start_epoch_secs(native));
        if distance <= START_MATCH_TOLERANCE_SECS {
            candidates.push((distance, process_index, native_index));
        }
    }
}
candidates.sort_unstable();

let mut claimed_processes = HashSet::new();
let mut claimed_natives = HashSet::new();
for (_distance, process_index, native_index) in candidates {
    if claimed_processes.contains(&process_index) || claimed_natives.contains(&native_index) {
        continue;
    }
    claimed_processes.insert(process_index);
    claimed_natives.insert(native_index);
    merge_process_into_native(&mut snapshot.sessions, process_index, native_index);
    removed.insert(process_index);
}
```

- [ ] **Step 4: Verify GREEN**

Run: `cargo test --lib live::correlation::tests:: -- --nocapture`

Expected: exact PID, start-time matching, conflicting PID, exit, and live-only filter tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/live/correlation.rs
git commit -m "fix: correlate native logs to live processes"
```

### Task 4: Normalize empty throughput and verify end to end

**Files:**
- Modify: `src/live/correlation.rs`
- Modify: `src/aggregate/part2.rs`
- Modify: `tests/aggregate.rs`

**Interfaces:**
- Produces: positive `0.0` for empty aggregate throughput in live and raw snapshots.

- [ ] **Step 1: Write the failing sign regression**

```rust
#[test]
fn empty_live_snapshot_uses_positive_zero_throughput() {
    let result = correlate_process_sessions(snapshot(Vec::new()));
    assert_eq!(result.total_tps, 0.0);
    assert!(result.total_tps.is_sign_positive());
}
```

Add the raw aggregator equivalent in `tests/aggregate.rs`:

```rust
#[test]
fn empty_snapshot_uses_positive_zero_throughput() {
    let snapshot = Aggregator::default().snapshot(Utc.timestamp_opt(1_784_639_940, 0).unwrap());
    assert_eq!(snapshot.total_tps, 0.0);
    assert!(snapshot.total_tps.is_sign_positive());
}
```

- [ ] **Step 2: Verify RED**

Run: `cargo test positive_zero_throughput -- --nocapture`

Expected: both empty sums are `-0.0`, so the sign assertions fail.

- [ ] **Step 3: Fold from positive zero**

Replace both throughput `.sum()` calls in `Aggregator::snapshot` and `recompute_summary` with:

```rust
.fold(0.0_f64, |total, value| total + value)
```

- [ ] **Step 4: Verify focused behavior**

Run: `cargo test positive_zero_throughput -- --nocapture`

Expected: the regression passes with positive zero.

- [ ] **Step 5: Run full verification**

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build
```

Expected: every command exits 0 with no failed test or lint warning.

- [ ] **Step 6: Check scope and commit**

Run: `git diff --check && git status --short`

Expected: only planned files are modified and no whitespace errors exist.

```bash
git add src/live/correlation.rs src/aggregate/part2.rs tests/aggregate.rs
git commit -m "fix: normalize empty live throughput"
```
