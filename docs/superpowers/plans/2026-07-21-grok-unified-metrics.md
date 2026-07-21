# Grok Unified Metrics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Populate Grok `PROJECT`, `NOW`, `AVG`, and `OUT` from exact `unified.jsonl` runtime telemetry while keeping one row per live Grok process.

**Architecture:** Discover Grok's shared unified log with the existing passive cursor. The Grok adapter maps each PID to its root session, folds child inference records into that root, and emits cumulative usage plus a generic exact reported-rate event. The existing aggregator converts those events into current, token-weighted average, and cumulative output metrics before the existing live-only filter runs.

**Tech Stack:** Rust 1.85, Serde JSON, Tokio file polling, Chrono, existing adapter/aggregator test suites.

## Global Constraints

- Use only exact Grok telemetry; never estimate tokens from text.
- Preserve one TUI row per live OS process.
- Keep `updates.jsonl` as the lifecycle and tool-state source.
- Add no dependency, process-tree inspection, or TUI column.
- Use TDD and verify every regression test fails before production edits.

---

### Task 1: Aggregate exact reported token rates

**Files:**
- Modify: `src/model.rs`
- Modify: `src/aggregate/part0.rs`
- Modify: `src/aggregate/part2.rs`
- Modify: `src/aggregate/part3.rs`
- Test: `tests/aggregate.rs`

**Interfaces:**
- Consumes: `TelemetryEvent`, `TurnRuntime`, and the existing two-second `AggregatorConfig::rate_window`.
- Produces: `EventKind::RateReported { output_tokens: u64, tokens_per_second: f64 }`, exact recent rate, token-weighted average rate, and summed output tokens.

- [ ] **Step 1: Write the failing aggregate test**

Append this test to `tests/aggregate.rs`:

```rust
#[test]
fn reported_rates_use_latest_exact_value_and_token_weighted_average() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::default();
    aggregate.apply(event(t0, EventKind::TurnStarted));
    aggregate.apply(event(
        t0 + Duration::seconds(1),
        EventKind::RateReported {
            output_tokens: 20,
            tokens_per_second: 10.0,
        },
    ));
    aggregate.apply(event(
        t0 + Duration::seconds(2),
        EventKind::RateReported {
            output_tokens: 30,
            tokens_per_second: 30.0,
        },
    ));

    let current = aggregate.snapshot(t0 + Duration::seconds(2));
    let session = &current.sessions[0];
    assert_eq!(session.current_tps.value, Some(30.0));
    assert!((session.turn_average_tps.value.unwrap() - (50.0 / 3.0)).abs() < 0.001);
    assert_eq!(session.current_tps.confidence, Confidence::Exact);
    assert_eq!(session.output_tokens, 50);

    let stale = aggregate.snapshot(t0 + Duration::seconds(5));
    assert_eq!(stale.sessions[0].current_tps.value, Some(0.0));
}
```

- [ ] **Step 2: Run the test to verify RED**

Run: `cargo test --test aggregate reported_rates_use_latest_exact_value_and_token_weighted_average -- --nocapture`

Expected: compilation fails because `EventKind::RateReported` does not exist.

- [ ] **Step 3: Add the minimal event and aggregation support**

Add the event variant in `src/model.rs` after `OutputDelta`:

```rust
RateReported {
    output_tokens: u64,
    tokens_per_second: f64,
},
```

Extend `RateSample` in `src/aggregate/part0.rs`:

```rust
struct RateSample {
    at: DateTime<Utc>,
    units: f64,
    confidence: Confidence,
    reported_tps: Option<f64>,
}
```

Keep `record_output` as the shared token accumulator and add a reported-rate method:

```rust
fn record_output(&mut self, at: DateTime<Utc>, tokens: Option<u64>, confidence: Confidence) {
    // existing timing setup remains unchanged
    let Some(tokens) = tokens else { return; };
    let units = tokens as f64;
    self.exact_delta_tokens = self.exact_delta_tokens.saturating_add(tokens);
    self.samples.push_back(RateSample {
        at,
        units,
        confidence,
        reported_tps: None,
    });
    self.token_output_units += units;
    self.token_output_confidence =
        lower_known_confidence(self.token_output_confidence, confidence);
}

fn record_reported_rate(
    &mut self,
    at: DateTime<Utc>,
    output_tokens: u64,
    tokens_per_second: f64,
    confidence: Confidence,
) {
    self.record_output(at, Some(output_tokens), confidence);
    if let Some(sample) = self.samples.back_mut() {
        sample.reported_tps = Some(tokens_per_second);
    }
}
```

Handle the new event in `src/aggregate/part2.rs` immediately after `OutputDelta`:

```rust
EventKind::RateReported {
    output_tokens,
    tokens_per_second,
} => {
    if tokens_per_second.is_finite() && *tokens_per_second > 0.0 {
        let turn = session.ensure_turn(&event);
        turn.record_reported_rate(
            event.occurred_at,
            *output_tokens,
            *tokens_per_second,
            event.confidence,
        );
    }
}
```

In `recent_rate`, prefer the newest exact rate in the active window before the existing delta calculation:

```rust
if let Some(sample) = within_window.last() {
    if let Some(rate) = sample.reported_tps {
        return Some((
            MetricValue::new(rate, sample.confidence),
            RateUnit::TokensPerSecond,
        ));
    }
}
```

In `average_rate`, use token-weighted inference duration when every token sample has a reported rate, otherwise retain the current elapsed-time calculation:

```rust
if !turn.samples.is_empty() && turn.samples.iter().all(|sample| sample.reported_tps.is_some()) {
    let seconds: f64 = turn
        .samples
        .iter()
        .map(|sample| sample.units / sample.reported_tps.unwrap())
        .sum();
    if seconds > 0.0 {
        return Some((
            MetricValue::new(
                turn.token_output_units / seconds,
                turn.token_output_confidence,
            ),
            RateUnit::TokensPerSecond,
        ));
    }
}
```

- [ ] **Step 4: Run the focused and full aggregate tests**

Run: `cargo test --test aggregate -- --nocapture`

Expected: all aggregate tests pass, including the existing delta-rate behavior.

- [ ] **Step 5: Commit**

```bash
git add src/model.rs src/aggregate/part0.rs src/aggregate/part2.rs src/aggregate/part3.rs tests/aggregate.rs
git commit -m "feat: aggregate exact reported token rates"
```

### Task 2: Parse Grok unified runtime telemetry

**Files:**
- Modify: `src/adapters/common.rs`
- Modify: `src/adapters/grok_build.rs`
- Create: `src/adapters/grok_build/parse_unified.rs`
- Test: `tests/adapters_grok_build.rs`

**Interfaces:**
- Consumes: `EventKind::RateReported`, `UsageFields`, and unified records with `ts`, `pid`, `sid`, `msg`, and `ctx`.
- Produces: `GrokBuildAdapter::parse_unified`, root-session metadata, root cumulative usage, and exact reported-rate events.

- [ ] **Step 1: Write the failing adapter test**

Append this test to `tests/adapters_grok_build.rs`:

```rust
#[test]
fn grok_unified_metrics_fold_child_inference_into_the_pid_root() {
    let mut adapter = adapter_for(ToolId::GrokBuild);
    let observed_at = Utc.timestamp_opt(1_784_505_900, 0).unwrap();
    let records = [
        serde_json::json!({
            "ts": "2026-07-21T14:30:56.958Z",
            "pid": 93524,
            "sid": "root-session",
            "src": "shell",
            "msg": "session created",
            "ctx": {"cwd": "/work/AquaTick"}
        }),
        serde_json::json!({
            "ts": "2026-07-21T14:31:09.280Z",
            "pid": 93524,
            "sid": "child-session",
            "src": "shell",
            "msg": "shell.turn.inference_done",
            "ctx": {
                "tokens_per_sec": 60.0,
                "completion_tokens": 12,
                "prompt_tokens": 100,
                "cached_prompt_tokens": 40,
                "reasoning_tokens": 4
            }
        }),
        serde_json::json!({
            "ts": "2026-07-21T14:31:15.782Z",
            "pid": 93524,
            "sid": "root-session",
            "src": "shell",
            "msg": "shell.turn.inference_done",
            "ctx": {
                "tokens_per_sec": 100.0,
                "completion_tokens": 20,
                "prompt_tokens": 180,
                "cached_prompt_tokens": 60,
                "reasoning_tokens": 6
            }
        }),
    ];
    let events = records
        .iter()
        .flat_map(|record| {
            adapter.parse_record(record, &AdapterContext::new("unified", observed_at))
        })
        .collect::<Vec<_>>();

    assert!(events.iter().all(|event| event.session_id == "root-session"));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::Metadata { cwd: Some(cwd), .. } if cwd == "/work/AquaTick"
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::RateReported { .. }))
            .count(),
        2
    );
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::Usage {
            input_tokens: Some(280),
            output_tokens: Some(32),
            cached_input_tokens: Some(100),
            reasoning_tokens: Some(10),
            cumulative: true,
            ..
        }
    )));
    assert_eq!(
        events.last().unwrap().occurred_at,
        at("2026-07-21T14:31:15.782Z")
    );
}
```

Use the existing `at` helper or add this local helper beside `parse_fixture`:

```rust
fn at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}
```

- [ ] **Step 2: Run the test to verify RED**

Run: `cargo test --test adapters_grok_build grok_unified_metrics_fold_child_inference_into_the_pid_root -- --nocapture`

Expected: the adapter emits no events for unified records.

- [ ] **Step 3: Implement unified parsing**

Add `&["ts"]` to the timestamp paths in `source_timestamp` in `src/adapters/common.rs`.

Extend `GrokBuildAdapter` in `src/adapters/grok_build.rs`:

```rust
#[derive(Default)]
pub struct GrokBuildAdapter {
    state: ParserState,
    pending_new_sessions: HashMap<String, String>,
    pending_prompts: HashMap<String, String>,
    retrying_sessions: HashSet<String>,
    unified_roots: HashMap<u32, String>,
    unified_usage: HashMap<String, UsageFields>,
}
```

Route unified records before headless and RPC records:

```rust
if value.get("msg").is_some() && value.get("pid").is_some() {
    return self.parse_unified(value, context);
}
```

Add the include:

```rust
include!("grok_build/parse_unified.rs");
```

Create `src/adapters/grok_build/parse_unified.rs` with:

```rust
impl GrokBuildAdapter {
    fn parse_unified(
        &mut self,
        value: &Value,
        context: &AdapterContext,
    ) -> Vec<TelemetryEvent> {
        let Some(pid) = first_u64_at(value, &[&["pid"]])
            .and_then(|pid| u32::try_from(pid).ok())
        else {
            return Vec::new();
        };
        let Some(source_session) = string_at(value, &["sid"]) else {
            return Vec::new();
        };

        match string_at(value, &["msg"]).unwrap_or_default() {
            "session created" => {
                self.unified_roots.insert(pid, source_session.to_owned());
                vec![
                    event(
                        self.tool(),
                        source_session,
                        None,
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::SessionStarted,
                    ),
                    event(
                        self.tool(),
                        source_session,
                        None,
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::Metadata {
                            cwd: string_at(value, &["ctx", "cwd"]).map(str::to_owned),
                            model: None,
                            provider: Some("xai".to_owned()),
                            pid: None,
                        },
                    ),
                ]
            }
            "shell.turn.inference_done" => {
                let session = self
                    .unified_roots
                    .get(&pid)
                    .map(String::as_str)
                    .unwrap_or(source_session)
                    .to_owned();
                let Some(output_tokens) = first_u64_at(
                    value,
                    &[&["ctx", "completion_tokens"]],
                ) else {
                    return Vec::new();
                };
                let Some(tokens_per_second) = value
                    .pointer("/ctx/tokens_per_sec")
                    .and_then(Value::as_f64)
                    .filter(|rate| rate.is_finite() && *rate > 0.0)
                else {
                    return Vec::new();
                };
                let delta = UsageFields {
                    input_tokens: first_u64_at(value, &[&["ctx", "prompt_tokens"]]),
                    output_tokens: Some(output_tokens),
                    cached_input_tokens: first_u64_at(
                        value,
                        &[&["ctx", "cached_prompt_tokens"]],
                    ),
                    reasoning_tokens: first_u64_at(
                        value,
                        &[&["ctx", "reasoning_tokens"]],
                    ),
                    context_window: None,
                };
                let total = self.unified_usage.entry(session.clone()).or_default();
                add_usage(total, delta);
                vec![
                    event(
                        self.tool(),
                        &session,
                        None,
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::RateReported {
                            output_tokens,
                            tokens_per_second,
                        },
                    ),
                    event(
                        self.tool(),
                        &session,
                        None,
                        value,
                        context,
                        Confidence::Exact,
                        total.into_event(true),
                    ),
                ]
            }
            _ => Vec::new(),
        }
    }
}

fn add_usage(total: &mut UsageFields, delta: UsageFields) {
    total.input_tokens = Some(
        total
            .input_tokens
            .unwrap_or_default()
            .saturating_add(delta.input_tokens.unwrap_or_default()),
    );
    total.output_tokens = Some(
        total
            .output_tokens
            .unwrap_or_default()
            .saturating_add(delta.output_tokens.unwrap_or_default()),
    );
    total.cached_input_tokens = Some(
        total
            .cached_input_tokens
            .unwrap_or_default()
            .saturating_add(delta.cached_input_tokens.unwrap_or_default()),
    );
    total.reasoning_tokens = Some(
        total
            .reasoning_tokens
            .unwrap_or_default()
            .saturating_add(delta.reasoning_tokens.unwrap_or_default()),
    );
}
```

- [ ] **Step 4: Run the Grok adapter tests**

Run: `cargo test --test adapters_grok_build -- --nocapture`

Expected: all Grok adapter tests pass and no private payload appears in normalized events.

- [ ] **Step 5: Commit**

```bash
git add src/adapters/common.rs src/adapters/grok_build.rs src/adapters/grok_build/parse_unified.rs tests/adapters_grok_build.rs
git commit -m "feat: ingest exact Grok unified metrics"
```

### Task 3: Discover the shared Grok unified log

**Files:**
- Modify: `src/registry.rs`
- Modify: `src/live/source_catalog.rs`

**Interfaces:**
- Consumes: `ToolDescriptor::resolve_session_roots` and `matches_session_source`.
- Produces: discovery of both Grok `updates.jsonl` and `unified.jsonl` files through the existing source cursor.

- [ ] **Step 1: Extend the failing source-catalog regression**

In `tool_specific_matchers_exclude_unrelated_json_and_accept_native_sources`, create the logs directory and files:

```rust
let grok_logs = home.join(".grok/logs");
fs::create_dir_all(&grok_logs).unwrap();
fs::write(grok_logs.join("unified.jsonl"), "{}\n").unwrap();
fs::write(grok_logs.join("random.jsonl"), "{}\n").unwrap();
```

Add assertions:

```rust
assert!(paths.contains(&PathBuf::from(".grok/logs/unified.jsonl")));
assert!(!paths.contains(&PathBuf::from(".grok/logs/random.jsonl")));
```

- [ ] **Step 2: Run the test to verify RED**

Run: `cargo test --lib tool_specific_matchers_exclude_unrelated_json_and_accept_native_sources -- --nocapture`

Expected: `unified.jsonl` is not discovered.

- [ ] **Step 3: Add the existing Grok logs root and exact file matcher**

Change the Grok descriptor in `src/registry.rs`:

```rust
session_roots: &["~/.grok/sessions", "~/.grok/logs"],
```

Change the Grok matcher in `src/live/source_catalog.rs`:

```rust
ToolId::GrokBuild => matches!(file_name.as_str(), "updates.jsonl" | "unified.jsonl"),
```

- [ ] **Step 4: Run source and registry tests**

Run: `cargo test source_catalog --lib -- --nocapture && cargo test --test registry -- --nocapture`

Expected: all source-catalog and registry tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/registry.rs src/live/source_catalog.rs
git commit -m "feat: discover Grok unified telemetry"
```

### Task 4: Verify the live Grok row end to end

**Files:**
- No production changes expected.

**Interfaces:**
- Consumes: the running Grok processes and their real `~/.grok/logs/unified.jsonl` records.
- Produces: one live row per Grok PID with exact project and token metrics.

- [ ] **Step 1: Run all automated verification**

Run:

```bash
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build
```

Expected: every command exits zero with no test failures or clippy warnings.

- [ ] **Step 2: Check the live snapshot**

Run: `./target/debug/llmeter once`

Expected for an active Grok process with unified records:

- exactly one row for that live PID;
- `OUT(tok)` is greater than zero after an `inference_done` record;
- `NOW(t/s)` is the exact latest reported rate inside two seconds, otherwise `0.0`;
- the expanded TUI shows the project basename from exact CWD metadata.

- [ ] **Step 3: Inspect the final diff**

Run: `git status --short && git diff --check && git log --oneline -6`

Expected: no uncommitted implementation changes, no whitespace errors, and the three implementation commits are present.
