use std::path::{Path, PathBuf};
#[cfg(not(unix))]
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::adapters::{adapter_for, Adapter, AdapterContext};
use crate::model::{TelemetryEvent, ToolId};

const CHECKPOINT_BYTES: u64 = 64;

#[derive(Debug, Default)]
pub struct CursorPoll {
    pub events: Vec<TelemetryEvent>,
    pub malformed_records: usize,
    pub bytes_read: u64,
    pub reset: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created_at: Option<SystemTime>,
}

impl FileIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                created_at: metadata.created().ok(),
            }
        }
    }
}

#[derive(Debug)]
struct LineCursor {
    path: PathBuf,
    bootstrap_bytes: u64,
    initialized: bool,
    offset: u64,
    pending: Vec<u8>,
    checkpoint: Vec<u8>,
    identity: Option<FileIdentity>,
}

#[derive(Debug, Default)]
struct LinePoll {
    lines: Vec<String>,
    bytes_read: u64,
    reset: bool,
}

impl LineCursor {
    fn new(path: impl Into<PathBuf>, bootstrap_bytes: u64) -> Self {
        Self {
            path: path.into(),
            bootstrap_bytes,
            initialized: false,
            offset: 0,
            pending: Vec::new(),
            checkpoint: Vec::new(),
            identity: None,
        }
    }

    async fn poll(&mut self) -> Result<LinePoll> {
        let mut file = fs::File::open(&self.path)
            .await
            .with_context(|| format!("open passive source {}", self.path.display()))?;
        let metadata = file
            .metadata()
            .await
            .with_context(|| format!("stat passive source {}", self.path.display()))?;
        let length = metadata.len();
        let identity = FileIdentity::from_metadata(&metadata);

        let mut reset = !self.initialized
            || self.identity.as_ref().is_some_and(|known| known != &identity)
            || length < self.offset;
        if !reset && !self.checkpoint_matches(&mut file).await? {
            reset = true;
        }

        let mut start = self.offset;
        let mut skip_partial_prefix = false;
        if reset {
            start = length.saturating_sub(self.bootstrap_bytes);
            skip_partial_prefix = start > 0;
            self.offset = start;
            self.pending.clear();
            self.checkpoint.clear();
            self.identity = Some(identity);
            self.initialized = true;
        }

        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut appended = Vec::new();
        file.read_to_end(&mut appended).await?;
        let bytes_read = appended.len() as u64;
        self.offset = start.saturating_add(bytes_read);

        if skip_partial_prefix {
            if let Some(newline) = appended.iter().position(|byte| *byte == b'\n') {
                appended.drain(..=newline);
            } else {
                appended.clear();
            }
        }

        let mut buffer = std::mem::take(&mut self.pending);
        buffer.extend_from_slice(&appended);
        let mut lines = Vec::new();
        let mut line_start = 0;
        for (index, byte) in buffer.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let mut line = &buffer[line_start..index];
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            lines.push(String::from_utf8_lossy(line).into_owned());
            line_start = index + 1;
        }
        self.pending.extend_from_slice(&buffer[line_start..]);
        self.refresh_checkpoint(&mut file).await?;

        Ok(LinePoll {
            lines,
            bytes_read,
            reset,
        })
    }

    async fn checkpoint_matches(&self, file: &mut fs::File) -> Result<bool> {
        if self.checkpoint.is_empty() || self.offset < self.checkpoint.len() as u64 {
            return Ok(true);
        }
        let start = self.offset - self.checkpoint.len() as u64;
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut current = vec![0; self.checkpoint.len()];
        match file.read_exact(&mut current).await {
            Ok(_) => Ok(current == self.checkpoint),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn refresh_checkpoint(&mut self, file: &mut fs::File) -> Result<()> {
        let length = CHECKPOINT_BYTES.min(self.offset);
        self.checkpoint.resize(length as usize, 0);
        if length == 0 {
            return Ok(());
        }
        file.seek(std::io::SeekFrom::Start(self.offset - length))
            .await?;
        file.read_exact(&mut self.checkpoint).await?;
        Ok(())
    }
}

pub struct SourceCursor {
    tool: ToolId,
    path: PathBuf,
    fallback_session_id: String,
    lines: LineCursor,
    adapter: Box<dyn Adapter>,
}

impl SourceCursor {
    pub fn new(tool: ToolId, path: impl Into<PathBuf>, bootstrap_bytes: u64) -> Self {
        let path = path.into();
        let fallback_session_id = fallback_session_id(&path, tool);
        Self {
            tool,
            lines: LineCursor::new(path.clone(), bootstrap_bytes),
            path,
            fallback_session_id,
            adapter: adapter_for(tool),
        }
    }

    pub fn tool(&self) -> ToolId {
        self.tool
    }

    pub async fn poll(&mut self, observed_at: &DateTime<Utc>) -> Result<CursorPoll> {
        let line_poll = self.lines.poll().await?;
        if line_poll.reset {
            self.adapter = adapter_for(self.tool);
        }

        let mut result = CursorPoll {
            bytes_read: line_poll.bytes_read,
            reset: line_poll.reset,
            ..CursorPoll::default()
        };
        for line in line_poll.lines {
            let normalized = normalize_wire_line(&line);
            if normalized.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(normalized) {
                Ok(value) => value,
                Err(_) => {
                    result.malformed_records += 1;
                    continue;
                }
            };
            let context = AdapterContext::new(self.fallback_session_id.clone(), *observed_at)
                .with_source_path(self.path.clone());
            let mut records = Vec::new();
            collect_json_records(&value, &mut records);
            for record in records {
                result
                    .events
                    .extend(self.adapter.parse_record(record, &context));
            }
        }
        Ok(result)
    }
}

pub struct JournalCursor {
    lines: LineCursor,
}

impl JournalCursor {
    pub fn new(path: impl Into<PathBuf>, bootstrap_bytes: u64) -> Self {
        Self {
            lines: LineCursor::new(path, bootstrap_bytes),
        }
    }

    pub fn path(&self) -> &Path {
        &self.lines.path
    }

    pub async fn poll(&mut self) -> Result<CursorPoll> {
        let line_poll = self.lines.poll().await?;
        let mut result = CursorPoll {
            bytes_read: line_poll.bytes_read,
            reset: line_poll.reset,
            ..CursorPoll::default()
        };
        for line in line_poll.lines {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<TelemetryEvent>(&line) {
                Ok(event) => result.events.push(event),
                Err(_) => result.malformed_records += 1,
            }
        }
        Ok(result)
    }
}

fn fallback_session_id(path: &Path, tool: ToolId) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}-passive", tool.as_str()))
}

fn normalize_wire_line(line: &str) -> &str {
    let line = line.trim_matches(|character| matches!(character, '\r' | '\n' | ' ' | '\t'));
    if line.starts_with(':') || line.starts_with("event:") || line.starts_with("id:") {
        return "";
    }
    line.strip_prefix("data:").map_or(line, str::trim_start)
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

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};

    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;
    use crate::model::{Confidence, EventKind};

    #[tokio::test]
    async fn line_cursor_reads_only_appended_complete_lines() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("events.jsonl");
        std::fs::write(&path, b"one\ntwo\n").unwrap();
        let mut cursor = LineCursor::new(&path, 1024);

        let first = cursor.poll().await.unwrap();
        assert_eq!(first.lines, vec!["one".to_owned(), "two".to_owned()]);
        assert!(first.reset);
        assert!(cursor.poll().await.unwrap().lines.is_empty());

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"three\npartial").unwrap();
        drop(file);
        assert_eq!(cursor.poll().await.unwrap().lines, vec!["three".to_owned()]);

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"-done\n").unwrap();
        drop(file);
        assert_eq!(
            cursor.poll().await.unwrap().lines,
            vec!["partial-done".to_owned()]
        );
        assert!(cursor.poll().await.unwrap().lines.is_empty());
    }

    #[tokio::test]
    async fn line_cursor_resets_on_truncate_or_checkpoint_rewrite() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("events.jsonl");
        std::fs::write(&path, b"alpha\nbeta\n").unwrap();
        let mut cursor = LineCursor::new(&path, 1024);
        cursor.poll().await.unwrap();

        std::fs::write(&path, b"new\n").unwrap();
        let truncated = cursor.poll().await.unwrap();
        assert!(truncated.reset);
        assert_eq!(truncated.lines, vec!["new".to_owned()]);

        std::fs::write(&path, b"same-a\nsame-b\n").unwrap();
        cursor.poll().await.unwrap();
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"diff-a\nsame-b\n").unwrap();
        drop(file);
        let rewritten = cursor.poll().await.unwrap();
        assert!(rewritten.reset);
        assert_eq!(
            rewritten.lines,
            vec!["diff-a".to_owned(), "same-b".to_owned()]
        );
    }

    #[tokio::test]
    async fn journal_cursor_replays_normalized_events_once_per_append() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("journal.jsonl");
        let at = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
        let first = TelemetryEvent::new(
            ToolId::Codex,
            "session-1",
            at,
            Confidence::Exact,
            EventKind::SessionStarted,
        );
        std::fs::write(&path, format!("{}\n", serde_json::to_string(&first).unwrap())).unwrap();
        let mut cursor = JournalCursor::new(&path, 1024);

        assert_eq!(cursor.poll().await.unwrap().events, vec![first]);
        assert!(cursor.poll().await.unwrap().events.is_empty());

        let second = TelemetryEvent::new(
            ToolId::Codex,
            "session-1",
            at + chrono::Duration::seconds(1),
            Confidence::Exact,
            EventKind::SessionEnded,
        );
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", serde_json::to_string(&second).unwrap()).unwrap();
        drop(file);
        assert_eq!(cursor.poll().await.unwrap().events, vec![second]);
    }

    #[tokio::test]
    async fn source_cursor_preserves_adapter_state_between_appends() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("updates.jsonl");
        std::fs::write(
            &path,
            "{\"timestamp\":\"2026-07-20T00:00:00Z\",\"hookEventName\":\"UserPromptSubmit\",\"sessionId\":\"grok-1\",\"promptId\":\"turn-1\"}\n",
        )
        .unwrap();
        let mut cursor = SourceCursor::new(ToolId::GrokBuild, &path, 4096);
        let at = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();

        let first = cursor.poll(&at).await.unwrap();
        assert!(first
            .events
            .iter()
            .any(|event| matches!(&event.kind, EventKind::TurnStarted)));
        assert!(cursor.poll(&at).await.unwrap().events.is_empty());

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{{\"timestamp\":\"2026-07-20T00:00:02Z\",\"hookEventName\":\"Stop\",\"sessionId\":\"grok-1\",\"promptId\":\"turn-1\"}}"
        )
        .unwrap();
        drop(file);
        let second = cursor.poll(&at).await.unwrap();
        assert!(second
            .events
            .iter()
            .any(|event| matches!(&event.kind, EventKind::TurnFinished { .. })));
    }
}
