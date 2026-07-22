# Direct Token Rate Calculation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Calculate Grok and Codex `NOW`/`AVG` token rates inside llmeter from token counts and timing data.

**Architecture:** Keep the existing `RateReported` normalized event and aggregator. Grok derives each rate from exact model timing fields; Codex tracks per-session response intervals and derives each rate from cumulative token growth and rollout timestamps.

**Tech Stack:** Rust 1.85, Chrono, Serde JSON, existing adapter and aggregator APIs.

## Global Constraints

- Do not add dependencies or reintroduce character throughput.
- Preserve cumulative usage ingestion even when a rate cannot be calculated.
- Keep the normalized event schema and TUI unchanged.
- Mark Grok calculated rates `Derived`; mark Codex fallback timing `Estimated`.

---

### Task 1: Calculate Grok Rates from Model Timing

**Files:**
- Modify: `src/adapters/grok_build/parse_unified.rs:35-104`
- Test: `tests/adapters_grok_build.rs:280-360`

**Interfaces:**
- Consumes: `ctx.completion_tokens: u64`, `ctx.model_elapsed_ms: u64`, optional `ctx.ttft_ms: u64`.
- Produces: existing `EventKind::RateReported { output_tokens: u64, tokens_per_second: f64 }` with `Confidence::Derived`.

- [ ] **Step 1: Write failing direct-calculation tests**

Add `model_elapsed_ms` and `ttft_ms` to the existing unified records, deliberately set `tokens_per_sec` to incorrect values, and assert the emitted rates are calculated values:

```rust
let rates = events
    .iter()
    .filter_map(|event| match event.kind {
        EventKind::RateReported {
            tokens_per_second,
            ..
        } => Some((tokens_per_second, event.confidence)),
        _ => None,
    })
    .collect::<Vec<_>>();
assert_eq!(
    rates,
    vec![
        (60.0, Confidence::Derived),
        (100.0, Confidence::Derived),
    ]
);
```

Add one record with `model_elapsed_ms <= ttft_ms` and assert it still emits cumulative `Usage` but no `RateReported`.

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test --test adapters_grok_build grok_unified -- --nocapture`

Expected: FAIL because the adapter still trusts `ctx.tokens_per_sec` and drops invalid-rate records before usage emission.

- [ ] **Step 3: Calculate the rate locally**

In `parse_unified`, retain usage construction and replace the provider-rate read with:

```rust
let rate = first_u64_at(value, &[&["ctx", "model_elapsed_ms"]])
    .and_then(|elapsed| {
        elapsed
            .checked_sub(first_u64_at(value, &[&["ctx", "ttft_ms"]]).unwrap_or_default())
    })
    .filter(|duration| *duration > 0 && output_tokens > 0)
    .map(|duration| output_tokens as f64 * 1_000.0 / duration as f64);
```

Always emit cumulative usage. Prepend `RateReported` only when `rate` is `Some`, using `Confidence::Derived`.

- [ ] **Step 4: Run the Grok tests and verify GREEN**

Run: `cargo test --test adapters_grok_build -- --nocapture`

Expected: all Grok adapter tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/adapters/grok_build/parse_unified.rs tests/adapters_grok_build.rs
git commit -m "fix: calculate Grok token rates locally"
```

### Task 2: Calculate Codex Rates from Rollout Boundaries

**Files:**
- Modify: `src/adapters/codex.rs:1-390`
- Test: `tests/adapters_phase2.rs:1-80`

**Interfaces:**
- Consumes: rollout timestamps, `task_started`, assistant-message/tool-call boundaries, `total_token_usage.output_tokens`, and `last_token_usage.output_tokens`.
- Produces: existing `EventKind::RateReported { output_tokens: u64, tokens_per_second: f64 }` with derived or estimated confidence.

- [ ] **Step 1: Write the failing Codex rate test**

Feed one adapter a task start, two model tool-call boundaries, delayed tool results, and two token-count records. Assert the rates use model intervals rather than tool completion times:

```rust
assert_eq!(
    rates,
    vec![
        (20, 10.0, Confidence::Derived),
        (10, 5.0, Confidence::Derived),
    ]
);
```

The first interval is `20 tokens / 2 seconds`; the second starts after the first tool result and is `10 tokens / 2 seconds`. Tool execution delays must not affect either result.

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test --test adapters_phase2 codex_calculates_rates_from_rollout_boundaries -- --nocapture`

Expected: FAIL because the Codex adapter emits no `RateReported` events.

- [ ] **Step 3: Add minimal per-session timing state**

Add a private adapter state:

```rust
#[derive(Default)]
struct CodexRateState {
    interval_started_at: Option<DateTime<Utc>>,
    model_output_at: Option<DateTime<Utc>>,
    cumulative_output_tokens: Option<u64>,
}
```

Store it in `CodexAdapter` as `rate: HashMap<String, CodexRateState>`.

- [ ] **Step 4: Record boundaries and calculate samples**

After normal event parsing, update the matching session state using `source_timestamp(value).unwrap_or(context.observed_at)`:

```rust
let elapsed_ms = end.signed_duration_since(start).num_milliseconds();
if output_tokens > 0 && elapsed_ms > 0 {
    output.push(event(
        ToolId::Codex,
        session,
        turn.as_deref(),
        value,
        context,
        confidence,
        EventKind::RateReported {
            output_tokens,
            tokens_per_second: output_tokens as f64 * 1_000.0 / elapsed_ms as f64,
        },
    ));
}
```

Use `last_token_usage.output_tokens` for the first fully observed inference. On later records use the non-decreasing cumulative difference. Reset rather than emit when the cumulative counter decreases. Clear the model boundary after each token count and make that token-count timestamp the next interval start.

- [ ] **Step 5: Run Codex tests and verify GREEN**

Run: `cargo test --test adapters_phase2 -- --nocapture`

Expected: all phase-2 adapter tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/adapters/codex.rs tests/adapters_phase2.rs
git commit -m "fix: calculate Codex token rates locally"
```

### Task 3: Verify Live Metrics and Repository Health

**Files:**
- Modify: none.
- Test: existing full suite and the current local Codex rollout.

**Interfaces:**
- Consumes: the completed Grok and Codex adapter changes.
- Produces: evidence that `NOW` and `AVG` are numeric and existing behavior remains intact.

- [ ] **Step 1: Run formatting, lint, and tests**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected: all tests pass, Clippy exits zero, and `git diff --check` is silent. Record any repository-wide pre-existing `cargo fmt --check` failures without broad unrelated formatting.

- [ ] **Step 2: Verify against the current Codex rollout**

Run:

```bash
cargo run --quiet -- json | jq '[.sessions[] | select(.tool == "codex") | {current_tps, turn_average_tps, rate_unit}]'
```

Expected: a fully observed active Codex session has numeric `current_tps` or stale `0.0`, numeric `turn_average_tps`, and `tokens_per_second` as its rate unit.

- [ ] **Step 3: Inspect final scope**

Run: `git status --short && git log -5 --oneline`

Expected: no uncommitted implementation files and separate Grok/Codex fix commits after the design and plan commits.
