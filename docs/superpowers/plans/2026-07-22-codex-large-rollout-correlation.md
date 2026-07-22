# Codex Large Rollout Correlation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correlate a running Codex process with its large rollout file even when the bootstrap tail excludes `session_meta`.

**Architecture:** Keep the existing tail bootstrap and correlation flow. Make `SourceCursor` derive Codex's fallback ID from the validated trailing UUID in the native rollout filename; the adapter still overrides it when `session_meta` is available.

**Tech Stack:** Rust, Tokio, `uuid`, existing unit-test harness

## Global Constraints

- Do not increase the 4 MiB bootstrap window or read the file header separately.
- Preserve the existing file-stem fallback for other tools and nonstandard Codex filenames.
- Add no dependency or abstraction.

---

### Task 1: Use the rollout filename UUID as Codex's fallback session ID

**Files:**
- Modify: `src/live/cursor.rs`
- Test: `src/live/cursor.rs`

**Interfaces:**
- Consumes: `fallback_session_id(path: &Path, tool: ToolId) -> String`
- Produces: the existing fallback ID contract, with a native UUID for standard Codex rollout filenames

- [ ] **Step 1: Write the failing large-rollout test**

Add a Tokio test that writes `session_meta`, a line larger than the 512-byte bootstrap, and a final `task_started` record. Assert the parsed tail event uses the filename UUID:

```rust
#[tokio::test]
async fn codex_large_rollout_uses_filename_session_id_when_meta_is_outside_bootstrap() {
    let temp = tempdir().unwrap();
    let session = "019f871c-a374-7af1-bdee-4ab563541fb2";
    let path = temp
        .path()
        .join(format!("rollout-2026-07-22T08-57-08-{session}.jsonl"));
    let contents = format!(
        "{{\"timestamp\":\"2026-07-21T23:57:08Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session}\"}}}}\n{{\"padding\":\"{}\"}}\n{{\"timestamp\":\"2026-07-22T00:00:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}\n",
        "x".repeat(1024)
    );
    std::fs::write(&path, contents).unwrap();
    let mut cursor = SourceCursor::new(ToolId::Codex, &path, 512);
    let at = Utc.timestamp_millis_opt(1_784_678_400_000).unwrap();

    let events = cursor.poll(&at).await.unwrap().events;

    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.session_id == session));
}
```

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test live::cursor::tests::codex_large_rollout_uses_filename_session_id_when_meta_is_outside_bootstrap -- --exact`

Expected: FAIL because the event session ID is the complete rollout file stem.

- [ ] **Step 3: Implement the minimal fallback extraction**

Import `uuid::Uuid`. In `fallback_session_id`, retain the file stem, then return its final 36 characters only when the tool is Codex and `Uuid::parse_str` validates them:

```rust
fn fallback_session_id(path: &Path, tool: ToolId) -> String {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty());
    if tool == ToolId::Codex {
        if let Some(session_id) = stem
            .and_then(|name| name.get(name.len().saturating_sub(36)..))
            .filter(|candidate| Uuid::parse_str(candidate).is_ok())
        {
            return session_id.to_owned();
        }
    }
    stem.map(str::to_owned)
        .unwrap_or_else(|| format!("{}-passive", tool.as_str()))
}
```

- [ ] **Step 4: Verify GREEN and regression suite**

Run: `cargo test live::cursor::tests::codex_large_rollout_uses_filename_session_id_when_meta_is_outside_bootstrap -- --exact`

Expected: PASS.

Run: `cargo test && cargo clippy --all-targets -- -D warnings`

Expected: all tests pass and Clippy exits 0.

- [ ] **Step 5: Verify the live sessions and commit**

Run: `cargo run --quiet -- json | jq '[.sessions[] | select(.tool == "codex") | {session_id,pid,state,output_tokens,current_tps,turn_average_tps,rate_unit}]'`

Expected: each running Codex PID correlates to a UUID session; active rollout rows have numeric NOW/AVG token rates.

```bash
git add src/live/cursor.rs
git commit -m "fix: correlate large Codex rollouts"
```
