# Live Session Correlation Fix Design

## Goal

Show exactly one TUI row per live, user-facing LLM process. Never show a native session whose OS process is no longer alive.

## Root Causes

- Orca Codex writes rollouts below `~/Library/Application Support/orca/codex-runtime-home/home/sessions`, while llmeter scans only `~/.codex/sessions`.
- Native Codex and Grok records usually have no PID, so ambiguous process/session correlation fails and the live filter discards their metrics.
- Process discovery currently timestamps placeholders at scan time instead of the process start time.
- `codex app-server` is a daemon, not a user session.

## Design

1. Add Orca's Codex session root and optional `CODEX_HOME` / `ORCA_CODEX_HOME` roots to the existing registry.
2. Exclude `codex app-server` from process discovery.
3. Timestamp process discovery events with the process's calculated start time while retaining the scan time as `observed_at`.
4. Correlate native sessions to live process placeholders in this order:
   - exact PID match;
   - otherwise, closest start time within 60 seconds.
5. Derive a native session's start time from its UUIDv7 session ID when available; otherwise use its aggregated `started_at` value.
6. Merge only one native session into each process placeholder. Once merged, the native row inherits the PID.
7. Keep the existing final live-process filter: rows with no live `(tool, pid)` pair are removed. An unmatched live process remains as a `NEW` placeholder, but unmatched or historical native sessions are never displayed.
8. Fold throughput from positive zero so an empty dashboard renders `0.0`, not `-0.0`.

## Data Flow

`ps` discovery produces one placeholder for each live user-facing process. Passive source adapters produce native session rows. Correlation assigns at most one native row to each placeholder using PID or start time. The final filter removes every row not backed by a currently live process, then the TUI renders the surviving rows.

## Failure Behavior

- If no native log matches a live process, keep that live process as a `NEW` row with unknown metrics.
- If multiple native logs are within the tolerance, choose the closest unclaimed start time deterministically.
- If a process exits, remove its row on the next process scan even when its log continues to exist.

## Tests

- Orca and environment-provided Codex roots resolve correctly.
- `codex app-server` is excluded while interactive Codex commands remain discoverable.
- Process discovery records the actual process start time.
- Multiple historical native sessions do not block matching the one whose UUIDv7 start is closest to the live process.
- Multiple processes claim at most one native session each.
- Unmatched native sessions are filtered; unmatched live process placeholders remain.
- Empty throughput is positive `0.0`.

## Non-goals

- No process-tree inspection, `lsof`, or new dependency.
- No display of recent-but-exited sessions.
- No changes to adapter wire formats or historical replay mode.
