# llmeter

A Rust TUI that shows TTFT, current/average output rate, tool runtime, and stall state for live LLM coding-agent sessions on one screen.

## Contents

1. Current scope
2. Install and run
3. Reading the metrics
4. Per-tool wiring
5. CLI commands
6. Architecture
7. Privacy
8. Known limitations

## 1. Current scope

Phase 1 and Phase 2 cover eight tools plus Grok Build, all on one normalized event model.

| Phase | Tool | Input surface | Auto-discovery |
|---|---|---|---|
| 1 | Pi | RPC/session JSONL | `~/.pi/agent/sessions/**/*.jsonl` |
| 1 | Factory Droid | stream JSON/JSON-RPC, hook | process |
| 1 | Gemini CLI | hook, telemetry JSON | process |
| 1 | Claude Code | hook, transcript JSONL | `~/.claude/projects/**/*.jsonl` |
| 2 | Codex CLI | rollout JSONL | `~/.codex/sessions/**/rollout-*.jsonl` |
| 2 | OpenCode | server/SSE/run JSON | process |
| 2 | Qwen Code | hook, telemetry JSON, daemon event | process |
| 2 | Kiro CLI | hook, ACP JSON-RPC | process |
| Extension | Grok Build | hook, streaming JSON, ACP, updates JSONL | `~/.grok/sessions/**/updates.jsonl` |

Automatic process discovery estimates session presence, PID, and project path. TTFT and output rate are computed only when structured events or a session file are attached.

## 2. Install and run

### Recommended (prebuilt binary)

Install the latest release into `~/.local/bin` (no Rust toolchain, no `sudo`):

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh | sh
```

Pin a version:

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh | LLMETER_VERSION=0.1.3 sh
```

Defaults and overrides:

- Install path: `~/.local/bin` (override with `INSTALL_DIR`)
- Ensure `~/.local/bin` is on your `PATH` if the installer prints a hint
- Manual downloads: [GitHub Releases](https://github.com/bengHak/llmeter/releases)

Supported platforms:

- macOS arm64 (`aarch64-apple-darwin`)
- Linux x86_64 (`x86_64-unknown-linux-gnu`)
- Linux arm64 (`aarch64-unknown-linux-gnu`)

Not supported in this installer: Windows; macOS Intel (x86_64). Binaries are not notarized—on macOS you may need to clear quarantine (`xattr -d com.apple.quarantine ~/.local/bin/llmeter`) if Gatekeeper blocks the binary.

Review before piping (optional):

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh -o install.sh
less install.sh
sh install.sh
```

The installer verifies the downloaded tarball against the release `SHA256SUMS` before installing.

### Developer path (from source)

Requires Rust 1.85 or later.

```bash
cargo build --release
./target/release/llmeter
```

After install, run `llmeter` with no extra setup: it discovers supported CLI processes and known session stores. On first pass it bootstraps recent history; after that it only processes newly appended bytes per file. `llmeter setup <tool>` is optional—it adds precise hook, RPC, and OTLP events rather than being required for discovery.

Print a one-shot JSON snapshot of running processes and known session files:

```bash
llmeter --once --json
```

Attach explicit JSONL sources:

```bash
llmeter --source codex:$HOME/.codex/sessions/2026/07/20/rollout-demo.jsonl
llmeter --source pi:/tmp/pi-session.jsonl \
        --source qwen:/tmp/qwen-telemetry.jsonl
```

Replay a historical file:

```bash
llmeter replay /tmp/pi-session.jsonl --tool pi --json
llmeter replay examples/normalized-session.jsonl --json
```

## 3. Reading the metrics

- `TTFT`: time from request submit to first output event
- `NOW`: output throughput over the last 2 seconds
- `AVG`: average throughput from first to last output of the current or latest turn
- `TOOL`: cumulative tool-call runtime
- `STALL`: time spent producing output with no new output for at least the default 2 seconds

Each metric shows its own confidence grade:

| Mark | Grade | Meaning |
|---|---|---|
| `●` | Exact | Directly instrumented timestamps or token counts |
| `◐` | Derived | Computed from exact values |
| `~` | Estimated | Based on character volume, process signals, or indirect events |
| `-` | Unknown | No usable data |

Rates are token-only (`tok/s`). Events without token counts still advance TTFT/stall timing, but do not invent a character throughput.

## 4. Per-tool wiring

See which connection methods work in your environment:

```bash
llmeter adapters
llmeter setup pi
llmeter setup claude
llmeter setup codex
llmeter setup grok-build
```

### Pi

Pi session JSONL is auto-discovered. You can also attach a file explicitly:

```bash
llmeter --source pi:/path/to/session.jsonl
```

### Factory Droid

Normalize structured output into the journal. `ingest` reads stdin line by line and flushes each event batch immediately, so a running `llmeter` in another terminal updates before the stream ends:

```bash
droid exec -o stream-json <args> \
  | llmeter ingest --tool droid --input -
```

### Gemini CLI and Qwen Code

Attach JSONL that a hook or telemetry exporter appends:

```bash
llmeter --source gemini:/tmp/gemini-events.jsonl
llmeter --source qwen:/tmp/qwen-events.jsonl
```

Or normalize hook-command stdin into the local journal:

```bash
llmeter hook --tool gemini
llmeter hook --tool qwen
```

### Claude Code

Transcript JSONL is discovered under the known default root. Lifecycle hooks can use this command sink:

```bash
llmeter hook --tool claude
```

### Codex CLI

Rollout JSONL is auto-discovered under the default session root:

```bash
llmeter --source codex:/path/to/rollout.jsonl
```

### OpenCode

Save server events, SSE, or `run --format json` output as JSONL and attach it:

```bash
llmeter --source opencode:/tmp/opencode-events.jsonl
```

### Kiro CLI

Attach hook or ACP wire JSONL:

```bash
llmeter --source kiro:/tmp/kiro-events.jsonl
llmeter hook --tool kiro
```

### Grok Build

Discovers running `grok`, `xai-grok-pager`, and `xai-grok-shell` processes and auto-reads interactive session files at `~/.grok/sessions/**/updates.jsonl`. Headless and ACP can be wired via wrapper:

```bash
# Print hook setup and connection commands
llmeter setup grok-build

# Headless streaming JSON
llmeter wrap --tool grok-build -- \
  grok --no-auto-update -p <prompt> --output-format streaming-json

# ACP stdio
llmeter wrap --tool grok-build -- \
  grok --no-auto-update agent stdio

# Load saved updates into the normalized journal
llmeter ingest --tool grok-build \
  --file ~/.grok/sessions/<project>/<session>/updates.jsonl
llmeter json
```

Grok prompts, responses, thinking content, and raw tool I/O are not stored in the journal—only character/byte deltas and token usage remain.

## 5. CLI commands

```text
llmeter [watch]
llmeter --once --json
llmeter replay <FILE> [--tool <TOOL>] [--json]
llmeter ingest --tool <TOOL> --input <FILE|-> [--output <JOURNAL>]
llmeter hook --tool <TOOL> [--output <JOURNAL>]
llmeter doctor [--json]
llmeter setup <TOOL>
llmeter adapters [--json]
```

Main global options:

```text
--source <TOOL:PATH>       repeatable
--journal <PATH>           normalized journal path
--no-auto-discover         disable process and default session-root discovery
--refresh-ms <MS>          TUI refresh interval, default 250ms
--process-scan-ms <MS>     process/session rescan interval, default 2000ms
--stall-threshold-ms <MS>  stall threshold, default 2000ms
```

TUI keys:

```text
j/k or ↑/↓  select session
p           pause/resume
r           refresh now
q or Esc    quit
```

## 6. Architecture

```text
process discovery ─┐
native JSONL tail ─┼─> tool adapter ─> TelemetryEvent ─> SessionAggregator
hook journal ──────┘                                      │
                                                          ├─> TUI
                                                          └─> JSON snapshot
```

The single Rust crate is split by responsibility:

- `src/model.rs`: normalized event and session snapshot models
- `src/adapters/`: parsers for nine tools
- `src/discovery.rs`: process and native session discovery
- `src/aggregate/`: TTFT / TPS / stall computation
- `src/live.rs`: stateful source index, incremental tail, process correlation
- `src/runtime.rs`: ingest, SSE, wrapper, and replay compatibility
- `src/tui.rs`: Ratatui dashboard

Tool parsers never compute metrics themselves. Every parser emits only `TelemetryEvent`s; one aggregator owns timing and state.

## 7. Privacy

The default journal stores only:

- session and turn identifiers
- event kind and timestamps
- token, character, and byte deltas
- model, project, and PID metadata
- tool name and call ID
- whether an error occurred (provider error text is stripped)

Prompts, response bodies, tool arguments, API keys, and raw error messages are not written to the normalized journal. On Unix, journal directories and files are created with `0700` and `0600` permissions, and existing journal permissions are tightened to `0600` on append. Security of original CLI transcript or rollout files follows each tool’s own settings.

## 8. Known limitations

- There are no live smoke tests against real user environments with external CLI binaries installed. Parsers are verified with fixtures and replay tests based on official event surfaces.
- There is no network server yet that subscribes directly to OpenCode SSE, Qwen daemon SSE, or Gemini OTLP. Today events are tailed as recorded JSONL or streamed as structured stdout via `ingest`.
- Process and native session rows merge only when they share the same PID or a clear 1:1 relationship. Ambiguous multi-candidate cases stay as separate rows to avoid wrong merges.
- If internal session file schemas change (Codex, Claude, etc.), the corresponding parser fixtures and mappings must be updated.
- Dynamic plugin ABI, remote hosts, Kubernetes, and a web dashboard are out of Phase 1–2 scope.
