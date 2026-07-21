# Grok Unified Metrics Design

## Goal

Populate `PROJECT`, `NOW`, `AVG`, and `OUT` for every live Grok process from Grok's exact runtime telemetry while preserving one row per live OS process.

## Evidence

Grok appends process-scoped telemetry to `~/.grok/logs/unified.jsonl`:

- `session created` records contain `pid`, root `sid`, and `ctx.cwd`.
- `shell.turn.inference_done` records contain `pid`, `sid`, `ctx.tokens_per_sec`, `ctx.completion_tokens`, `ctx.prompt_tokens`, `ctx.cached_prompt_tokens`, `ctx.reasoning_tokens`, and `ctx.ttft_ms`.
- For the reproduced child session, summing nine `inference_done` records exactly matches its final `turn_completed` usage: 268,592 input, 3,795 output, 227,200 cached input, and 2,360 reasoning tokens.

`updates.jsonl` remains the source for lifecycle, output timing, tool spans, and final turn boundaries. It does not contain per-inference token rates.

## Design

1. Discover `~/.grok/logs/unified.jsonl` as an additional Grok source using the existing source cursor and adapter.
2. Recognize `session created`, remember the root session ID for its PID, and emit exact CWD metadata for that root session.
3. Recognize `shell.turn.inference_done` and route it to the remembered root session for the PID. This folds subagent inference into its one live parent-process row instead of exposing child rows.
4. Emit exact cumulative usage for the root session by summing each inference record's input, output, cached-input, and reasoning deltas in the Grok adapter.
5. Add one generic reported-rate event carrying the inference's exact output-token count and `tokens_per_sec` value.
6. Aggregate reported rates as follows:
   - `NOW` is the most recent exact reported rate inside the existing rate window, then `0.0` when stale.
   - `AVG` is the token-weighted exact rate: total reported output tokens divided by the sum of `output_tokens / tokens_per_sec` for each inference.
   - `OUT` is the summed exact output-token count. The final `turn_completed` total remains a reconciliation source and must not double count it.
7. Keep process discovery and the current PID-backed live filter as the liveness authority. Unified-log records for exited PIDs remain hidden.

## Data Flow

`unified.jsonl` → Grok adapter → root session selected by PID → exact CWD, cumulative usage, and reported-rate events → aggregator → existing process correlation and live-only filter → one TUI row.

`updates.jsonl` continues in parallel through the same adapter and aggregator for turn state and tool activity.

## Failure Behavior

- Ignore malformed unified records and records without a usable session ID or numeric usage.
- If the cursor starts after a process's `session created` record, attribute inference to its native `sid`; the existing UUIDv7 correlation still decides whether it belongs to a live row.
- Ignore non-positive or non-finite reported rates rather than corrupting aggregate metrics.
- Never estimate tokens from characters or bytes.

## Tests

- Grok source discovery accepts `unified.jsonl` and still rejects unrelated log files.
- A `session created` fixture emits root-session CWD metadata.
- Root and child `inference_done` fixtures sharing one PID produce one root session with exact summed usage.
- Reported `NOW` uses the latest exact rate and becomes zero outside the rate window.
- Reported `AVG` is token-weighted.
- Final usage reconciliation does not double count output tokens.
- Existing live-process filtering continues to hide sessions with no live PID.

## Non-goals

- No tokenizer, token estimation, file watching dependency, or process-tree inspection.
- No TUI column or wire-protocol redesign beyond the single generic reported-rate event.
- No display of completed child sessions as separate live rows.
