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

Run the interactive dashboard:

```bash
llmeter
```

Print a one-shot terminal table or JSON snapshot of discovered sessions:

```bash
# Formatted table snapshot
llmeter once

# Machine-readable JSON snapshot
llmeter json
```

Inspect discovered CLI executables, active processes, and session roots:

```bash
llmeter doctor
```

Replay a normalized journal file:

```bash
# Interactive TUI playback
llmeter replay examples/normalized-session.jsonl

# Snapshot dump from journal
llmeter replay examples/normalized-session.jsonl --json
```

## 3. Reading the metrics

- `TTFT`: time from request submit to first output event
- `NOW`: output throughput over the last 2 seconds
- `AVG`: average throughput from first to last output of the current or latest turn
- `TOOL`: cumulative tool-call runtime
- `STALL`: time spent producing output with no new output for at least the default 2.5 seconds

Each metric shows its own confidence grade:

| Mark | Grade | Meaning |
|---|---|---|
| `●` | Exact | Directly instrumented timestamps or token counts |
| `◐` | Derived | Computed from exact values |
| `~` | Estimated | Based on character volume, process signals, or indirect events |
| `-` | Unknown | No usable data |

Rates default to token throughput (`tok/s` or `t/s`). When tools emit only character or byte deltas without token metadata (e.g. Grok Build, raw streams), throughput is measured in characters (`c/s` or `char/s`) or bytes (`B/s`).

## 4. Per-tool wiring

Diagnose supported tools and connection paths in your environment:

```bash
llmeter doctor
```

Print safe integration snippets or hook configurations:

```bash
llmeter setup claude
llmeter setup droid
llmeter setup gemini
llmeter setup kiro
llmeter setup pi
llmeter setup codex
llmeter setup opencode
llmeter setup qwen
llmeter setup grok-build
```

### Pi

Pi session JSONL files are auto-discovered under `~/.pi/agent/sessions`. You can also wrap an RPC session or ingest past session logs:

```bash
# Wrap an interactive or RPC child process
llmeter wrap --tool pi -- pi --mode rpc

# Ingest saved session JSONL into the journal
llmeter ingest --tool pi --file ~/.pi/agent/sessions/<session>.jsonl
```

### Factory Droid

Normalize structured output or hook events into the journal. `ingest` reads stdin line by line and flushes each event batch immediately:

```bash
droid exec -o stream-json <args> \
  | llmeter ingest --tool droid

# Or configure command hooks
llmeter setup droid
llmeter hook --tool droid
```

### Gemini CLI and Qwen Code

Attach JSONL or ingest from hook commands:

```bash
# Ingest an exported log file
llmeter ingest --tool gemini --file /tmp/gemini-events.jsonl
llmeter ingest --tool qwen --file /tmp/qwen-events.jsonl

# Or normalize hook-command stdin into the local journal
llmeter hook --tool gemini
llmeter hook --tool qwen
```

### Claude Code

Transcript JSONL is automatically discovered under `~/.claude/projects`. Lifecycle hooks can use the command sink:

```bash
# Check hook configuration
llmeter setup claude

# Hook receiver
llmeter hook --tool claude
```

### Codex CLI

Interactive rollout JSONL is auto-discovered under `~/.codex/sessions`. For headless exec, wrap the command:

```bash
llmeter wrap --tool codex -- codex exec --json <prompt>
```

### OpenCode

Connect to OpenCode's live Server-Sent Events (SSE) endpoint or wrap CLI execution:

```bash
# Connect to running opencode server SSE
llmeter connect --tool opencode --url http://127.0.0.1:4096/global/event

# Or ingest saved server events
llmeter ingest --tool opencode --file /tmp/opencode-events.jsonl
```

### Kiro CLI

Attach hook or ACP wire:

```bash
# ACP stdio wrapper
llmeter wrap --tool kiro -- kiro-cli acp

# Hook command receiver
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
llmeter [tui]                                  Run live terminal dashboard (default)
llmeter once                                   Print one human-readable snapshot table
llmeter json                                   Print one JSON snapshot
llmeter doctor                                 Show executable, process, session-root, and journal diagnostics
llmeter setup <TOOL> [--binary <PATH>]         Print safe integration snippets for a tool
llmeter hook --tool <TOOL>                     Receive command-hook JSON payload from stdin
llmeter wrap --tool <TOOL> -- <COMMAND...>     Proxy a child process while measuring its stream
llmeter connect [--tool <TOOL>] [--url <URL>]  Connect to a tool's SSE endpoint (default: OpenCode)
llmeter ingest --tool <TOOL> [--file <PATH>]   Parse JSONL records into normalized journal (reads stdin if omitted)
llmeter replay <FILE> [--json]                 Replay a normalized journal in TUI or print JSON snapshot
```

Global options:

```text
--data-dir <DIR>   Override base data directory (default: ~/.local/share/llmeter or OS local data, env: LLMETER_DATA_DIR)
```

Interactive TUI controls:

```text
Tab          Switch focus between panels (Active Sessions, Run Ledger, Inspector)
j/k or ↓/↑   Navigate sessions / scroll inspector
s            Cycle sort mode (State -> TPS -> TTFT)
Enter        Inspect selected session details / close inspector
q / Esc      Quit (also supports Q, Ctrl+C, or Korean 'ㅂ')
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
- Direct network subscriptions currently support SSE endpoints (`llmeter connect` for OpenCode). Gemini OTLP or custom daemon telemetry require exporting as JSONL or streaming via `ingest`.
- Process and native session rows merge only when they share the same PID or a clear 1:1 relationship. Ambiguous multi-candidate cases stay as separate rows to avoid wrong merges.
- If internal session file schemas change (Codex, Claude, etc.), the corresponding parser fixtures and mappings must be updated.
- Dynamic plugin ABI, remote hosts, Kubernetes, and a web dashboard are out of Phase 1–2 scope.
