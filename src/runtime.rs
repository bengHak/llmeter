use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::fs;
use tokio::io::{
    self, AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader,
};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::adapters::{adapter_for, Adapter, AdapterContext};
use crate::aggregate::Aggregator;
use crate::discovery::{detect_processes, discover_session_files};
use crate::journal::Journal;
use crate::model::{AppSnapshot, Confidence, EventKind, TelemetryEvent, ToolId};

const MAX_PASSIVE_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PASSIVE_FILES: usize = 64;

#[derive(Clone, Copy, Debug, Default)]
pub struct IngestStats {
    pub records: usize,
    pub events: usize,
    pub malformed_records: usize,
}

pub async fn ingest_reader<R>(
    tool: ToolId,
    reader: R,
    fallback_session_id: impl Into<String>,
    source_path: Option<PathBuf>,
    journal: &Journal,
) -> Result<IngestStats>
where
    R: AsyncBufRead + Unpin,
{
    let mut adapter = adapter_for(tool);
    ingest_with_adapter(
        &mut *adapter,
        reader,
        fallback_session_id.into(),
        source_path,
        journal,
    )
    .await
}

async fn ingest_with_adapter<R>(
    adapter: &mut dyn Adapter,
    mut reader: R,
    fallback_session_id: String,
    source_path: Option<PathBuf>,
    journal: &Journal,
) -> Result<IngestStats>
where
    R: AsyncBufRead + Unpin,
{
    let mut stats = IngestStats::default();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .await
            .context("read telemetry input")?;
        if read == 0 {
            break;
        }
        let trimmed = normalize_wire_line(&line);
        if trimmed.is_empty() {
            continue;
        }
        stats.records += 1;
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => {
                stats.malformed_records += 1;
                continue;
            }
        };
        stats.events += parse_value_and_append(
            adapter,
            &value,
            &fallback_session_id,
            source_path.clone(),
            journal,
        )
        .await?;
    }
    Ok(stats)
}

pub async fn ingest_file(
    tool: ToolId,
    path: &Path,
    fallback_session_id: impl Into<String>,
    journal: &Journal,
) -> Result<IngestStats> {
    let file = fs::File::open(path)
        .await
        .with_context(|| format!("open input {}", path.display()))?;
    ingest_reader(
        tool,
        BufReader::new(file),
        fallback_session_id,
        Some(path.to_path_buf()),
        journal,
    )
    .await
}

pub async fn connect_sse(
    tool: ToolId,
    url: &str,
    fallback_session_id: impl Into<String>,
    journal: &Journal,
) -> Result<IngestStats> {
    let response = reqwest::Client::new()
        .get(url)
        .header("accept", "text/event-stream")
        .send()
        .await
        .with_context(|| format!("connect to {url}"))?
        .error_for_status()
        .with_context(|| format!("SSE endpoint rejected request: {url}"))?;
    let fallback_session_id = fallback_session_id.into();
    let mut stream = response.bytes_stream();
    let mut adapter = adapter_for(tool);
    let mut decoder = SseDecoder::default();
    let mut stats = IngestStats::default();

    while let Some(chunk) = stream.next().await {
        for payload in decoder.push(&chunk.context("read SSE chunk")?) {
            ingest_sse_payload(
                &mut *adapter,
                &payload,
                &fallback_session_id,
                journal,
                &mut stats,
            )
            .await?;
        }
    }
    for payload in decoder.finish() {
        ingest_sse_payload(
            &mut *adapter,
            &payload,
            &fallback_session_id,
            journal,
            &mut stats,
        )
        .await?;
    }
    Ok(stats)
}

async fn ingest_sse_payload(
    adapter: &mut dyn Adapter,
    payload: &str,
    fallback_session_id: &str,
    journal: &Journal,
    stats: &mut IngestStats,
) -> Result<()> {
    let payload = payload.trim();
    if payload.is_empty() {
        return Ok(());
    }
    stats.records += 1;
    let value: Value = match serde_json::from_str(payload) {
        Ok(value) => value,
        Err(_) => {
            stats.malformed_records += 1;
            return Ok(());
        }
    };
    stats.events +=
        parse_value_and_append(adapter, &value, fallback_session_id, None, journal).await?;
    Ok(())
}

#[derive(Debug, Default)]
struct SseDecoder {
    pending: Vec<u8>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let bytes: Vec<_> = self.pending.drain(..=position).collect();
            let line = String::from_utf8_lossy(&bytes);
            self.consume_line(&line, &mut events);
        }
        events
    }

    fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if !self.pending.is_empty() {
            let bytes = std::mem::take(&mut self.pending);
            let line = String::from_utf8_lossy(&bytes);
            self.consume_line(&line, &mut events);
        }
        self.flush_event(&mut events);
        events
    }

    fn consume_line(&mut self, line: &str, events: &mut Vec<String>) {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            self.flush_event(events);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        if line == "data" {
            self.data_lines.push(String::new());
            return;
        }
        if let Some(data) = line.strip_prefix("data:") {
            self.data_lines
                .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
        }
    }

    fn flush_event(&mut self, events: &mut Vec<String>) {
        if self.data_lines.is_empty() {
            return;
        }
        events.push(self.data_lines.join("\n"));
        self.data_lines.clear();
    }
}

pub async fn wrap_command(tool: ToolId, command: Vec<OsString>, journal: Journal) -> Result<i32> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| anyhow!("wrap requires a command after --"))?;
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", program.to_string_lossy()))?;
    let pid = child.id().unwrap_or_default();
    let fallback_session = format!("{}-process-{pid}", tool.as_str());
    let adapter: Arc<Mutex<Box<dyn Adapter>>> = Arc::new(Mutex::new(adapter_for(tool)));

    let child_stdin = child
        .stdin
        .take()
        .context("wrapped child stdin unavailable")?;
    let stdin_adapter = Arc::clone(&adapter);
    let stdin_journal = journal.clone();
    let stdin_session = fallback_session.clone();
    let stdin_task = tokio::spawn(async move {
        proxy_stdin(child_stdin, stdin_adapter, stdin_journal, stdin_session).await
    });

    let child_stderr = child
        .stderr
        .take()
        .context("wrapped child stderr unavailable")?;
    let stderr_task = tokio::spawn(async move {
        let mut child_stderr = child_stderr;
        let mut stderr = io::stderr();
        io::copy(&mut child_stderr, &mut stderr).await
    });

    let child_stdout = child
        .stdout
        .take()
        .context("wrapped child stdout unavailable")?;
    let mut lines = BufReader::new(child_stdout).lines();
    let mut stdout = io::stdout();
    while let Some(line) = lines.next_line().await.context("read wrapped stdout")? {
        stdout.write_all(line.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
        parse_shared_line(Arc::clone(&adapter), &journal, &fallback_session, &line).await?;
    }

    let status = child.wait().await.context("wait for wrapped command")?;
    stdin_task.abort();
    let _ = stderr_task.await;
    Ok(status.code().unwrap_or(1))
}

async fn proxy_stdin(
    mut child_stdin: tokio::process::ChildStdin,
    adapter: Arc<Mutex<Box<dyn Adapter>>>,
    journal: Journal,
    fallback_session: String,
) -> Result<()> {
    let mut lines = BufReader::new(io::stdin()).lines();
    while let Some(line) = lines.next_line().await.context("read wrapper stdin")? {
        child_stdin.write_all(line.as_bytes()).await?;
        child_stdin.write_all(b"\n").await?;
        child_stdin.flush().await?;
        parse_shared_line(Arc::clone(&adapter), &journal, &fallback_session, &line).await?;
    }
    child_stdin.shutdown().await?;
    Ok(())
}

async fn parse_shared_line(
    adapter: Arc<Mutex<Box<dyn Adapter>>>,
    journal: &Journal,
    fallback_session: &str,
    line: &str,
) -> Result<usize> {
    let trimmed = normalize_wire_line(line);
    if trimmed.is_empty() {
        return Ok(0);
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return Ok(0);
    };
    let mut adapter = adapter.lock().await;
    parse_value_and_append(&mut **adapter, &value, fallback_session, None, journal).await
}

async fn parse_value_and_append(
    adapter: &mut dyn Adapter,
    value: &Value,
    fallback_session_id: &str,
    source_path: Option<PathBuf>,
    journal: &Journal,
) -> Result<usize> {
    let mut records = Vec::new();
    collect_json_records(value, &mut records);
    let mut count = 0;
    for record in records {
        let mut context = AdapterContext::new(fallback_session_id, Utc::now());
        if let Some(path) = source_path.as_ref() {
            context = context.with_source_path(path.clone());
        }
        let events = adapter.parse_record(record, &context);
        count += journal.append_all(events.iter()).await?;
    }
    Ok(count)
}

fn collect_json_records<'a>(value: &'a Value, output: &mut Vec<&'a Value>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_json_records(value, output);
            }
        }
        _ => output.push(value),
    }
}

pub async fn load_snapshot(data_dir: &Path) -> Result<AppSnapshot> {
    let now = Utc::now();
    let journal = Journal::new(data_dir.join("events.jsonl"));
    let mut events = journal.read_all().await?;

    if let Ok(processes) = detect_processes() {
        for process in processes {
            let Some(tool) = process.tool else {
                continue;
            };
            events.push(TelemetryEvent::new(
                tool,
                format!("process-{}", process.pid),
                now,
                Confidence::Derived,
                EventKind::SessionDiscovered {
                    pid: Some(process.pid),
                    cwd: None,
                    model: None,
                },
            ));
        }
    }

    if let Some(home) = dirs::home_dir() {
        if let Ok(files) = discover_session_files(&home, 6, MAX_PASSIVE_FILES) {
            for discovered in files {
                if let Ok(mut passive_events) =
                    parse_passive_file(discovered.tool, &discovered.path).await
                {
                    events.append(&mut passive_events);
                }
            }
        }
    }

    Ok(snapshot_from_events(events, now))
}

pub async fn snapshot_from_journal(path: &Path) -> Result<AppSnapshot> {
    let events = Journal::new(path).read_all().await?;
    let snapshot_at = events
        .iter()
        .map(|event| event.occurred_at)
        .max()
        .unwrap_or_else(Utc::now);
    Ok(snapshot_from_events(events, snapshot_at))
}

fn snapshot_from_events(
    mut events: Vec<TelemetryEvent>,
    snapshot_at: chrono::DateTime<Utc>,
) -> AppSnapshot {
    events.sort_by_key(|event| event.occurred_at);
    let mut aggregate = Aggregator::default();
    aggregate.apply_all(events);
    aggregate.snapshot(snapshot_at)
}

async fn parse_passive_file(tool: ToolId, path: &Path) -> Result<Vec<TelemetryEvent>> {
    let mut file = fs::File::open(path).await?;
    let length = file.metadata().await?.len();
    let start = length.saturating_sub(MAX_PASSIVE_FILE_BYTES);
    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start)).await?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await?;
    if start > 0 {
        if let Some(position) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=position);
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let fallback = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("passive-session");
    let mut adapter = adapter_for(tool);
    let mut output = Vec::new();

    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        let context = AdapterContext::new(fallback, Utc::now()).with_source_path(path);
        let mut records = Vec::new();
        collect_json_records(&value, &mut records);
        for record in records {
            output.extend(adapter.parse_record(record, &context));
        }
        return Ok(output);
    }

    for line in text.lines() {
        let trimmed = normalize_wire_line(line);
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let context = AdapterContext::new(fallback, Utc::now()).with_source_path(path);
        if let Value::Array(values) = value {
            for value in values {
                output.extend(adapter.parse_record(&value, &context));
            }
        } else {
            output.extend(adapter.parse_record(&value, &context));
        }
    }
    Ok(output)
}

fn normalize_wire_line(line: &str) -> &str {
    let line = line.trim_matches(|character| matches!(character, '\r' | '\n' | ' ' | '\t'));
    if line.starts_with(':') || line.starts_with("event:") || line.starts_with("id:") {
        return "";
    }
    line.strip_prefix("data:").map_or(line, str::trim_start)
}

#[cfg(test)]
mod tests {
    use super::SseDecoder;

    #[test]
    fn sse_decoder_handles_chunk_boundaries_multiline_data_and_eof() {
        let mut decoder = SseDecoder::default();

        assert!(decoder
            .push(b": keepalive\r\nevent: message\r\ndata: {\"type\":\"session\",\r\n")
            .is_empty());
        let first = decoder.push(b"data: \"ok\":true}\r\n\r\n");
        assert_eq!(first, vec!["{\"type\":\"session\",\n\"ok\":true}"]);

        assert!(decoder.push(b"data: {\"type\":\"tail\"}").is_empty());
        assert_eq!(decoder.finish(), vec!["{\"type\":\"tail\"}"]);
    }
}
