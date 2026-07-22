# Direct Token Rate Calculation Design

## Goal

Populate `NOW` and `AVG` for Grok and Codex by calculating token rates inside llmeter instead of trusting a provider-reported rate or leaving the metrics unknown.

## Grok

For each `shell.turn.inference_done` record, calculate the generation duration as:

```text
generation_ms = model_elapsed_ms - ttft_ms_or_zero
tokens_per_second = completion_tokens / (generation_ms / 1000)
```

Emit the result through the existing `RateReported` event with derived confidence. Ignore the source `tokens_per_sec` value. Continue emitting cumulative usage exactly as today.

Grok omits `ttft_ms` when it is unavailable or zero, so treat a missing TTFT as zero. Skip the rate when `completion_tokens` is zero, `model_elapsed_ms` is missing, or the calculated duration is not positive. Usage ingestion must still continue.

## Codex

Codex rollout records expose exact cumulative and last-inference token usage but no generation duration or TPS. Calculate an observed model-response rate from exact token deltas and rollout timestamps:

1. Start an inference interval at `task_started`, or at the previous `token_count` after a tool result.
2. Track the latest model-output boundary from an assistant message or tool-call request.
3. At `token_count`, subtract the previous cumulative output-token total and divide that delta by the interval ending at the tracked model-output boundary. For the first inference in a fully observed turn, use `last_token_usage.output_tokens`.
4. Emit the result through the existing `RateReported` event with derived confidence.
5. Preserve the cumulative `Usage` event for `OUT` and context metrics.

Ending the interval at the tool-call request and starting the next interval after its result keeps known tool execution time out of the model rate. If no model-output boundary is available, use the `token_count` timestamp and mark the result estimated.

The first `token_count` without an earlier interval becomes a baseline only. A decreasing cumulative counter resets the baseline. Zero-token or non-positive-duration intervals do not emit a rate.

## Aggregation and UI

Reuse the existing `RateReported` aggregation:

- `NOW` is the newest calculated rate within the two-second window, then `0.0` when stale.
- `AVG` is token-weighted across calculated inference samples.
- No TUI or normalized-event schema change is required.

## Tests

- Grok calculates TPS from token count, model elapsed time, and TTFT even when the supplied `tokens_per_sec` is wrong.
- Grok skips invalid timing while retaining usage.
- Codex emits a calculated rate from cumulative token growth and model boundaries.
- Codex excludes a known tool span from the measured interval.
- Codex resets cleanly when counters decrease or required boundaries are missing.
- Existing aggregate tests continue to cover `NOW`, stale zero, token-weighted `AVG`, and no token double counting.

## Non-goals

- Reintroducing character throughput.
- Adding a tokenizer or dependency.
- Claiming Codex's observed response rate is raw decoder throughput; its confidence marker communicates the weaker timing surface.
- Renaming `RateReported` or changing the journal schema for this fix.
