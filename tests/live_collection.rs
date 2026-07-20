use std::io::Write;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use llmeter::live::{LiveCollector, LiveCollectorConfig};
use llmeter::model::{Confidence, EventKind, SessionState, TelemetryEvent, ToolId};
use tempfile::tempdir;

fn passive_config(home: std::path::PathBuf) -> LiveCollectorConfig {
    LiveCollectorConfig {
        home: Some(home),
        source_scan_interval: Duration::ZERO,
        process_scan_interval: Duration::ZERO,
        max_depth: 8,
        max_sources_per_tool: 16,
        max_sources_total: 64,
        source_bootstrap_bytes: 1024 * 1024,
        journal_bootstrap_bytes: 1024 * 1024,
        scan_processes: false,
    }
}

#[tokio::test]
async fn unchanged_native_source_is_not_applied_twice_and_append_is_incremental() {
    let temp = tempdir().unwrap();
    let home = temp.path().join("home");
    let data_dir = temp.path().join("llmeter-data");
    let source_dir = home.join(".grok/sessions/project/session-1");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let source = source_dir.join("updates.jsonl");
    std::fs::write(
        &source,
        "{\"timestamp\":\"2026-07-20T00:00:00Z\",\"hookEventName\":\"UserPromptSubmit\",\"sessionId\":\"grok-1\",\"promptId\":\"turn-1\"}\n",
    )
    .unwrap();

    let mut collector = LiveCollector::with_config(&data_dir, passive_config(home));
    let at = Utc.timestamp_millis_opt(1_784_505_601_000).unwrap();
    let first = collector.refresh_at(at).await.unwrap();
    let applied_after_first = collector.stats().events_applied;
    let bytes_after_first = collector.stats().bytes_read;
    assert_eq!(applied_after_first, 1);
    assert!(bytes_after_first > 0);
    assert_eq!(first.sessions.len(), 1);
    assert_eq!(collector.stats().source_count, 1);

    let second = collector.refresh_at(at).await.unwrap();
    assert_eq!(collector.stats().events_applied, applied_after_first);
    assert_eq!(collector.stats().bytes_read, bytes_after_first);
    assert_eq!(second.sessions.len(), 1);

    let mut file = std::fs::OpenOptions::new().append(true).open(&source).unwrap();
    writeln!(
        file,
        "{{\"timestamp\":\"2026-07-20T00:00:01Z\",\"hookEventName\":\"PreToolUse\",\"sessionId\":\"grok-1\",\"promptId\":\"turn-1\",\"toolUseId\":\"tool-1\",\"toolName\":\"run_terminal_command\"}}"
    )
    .unwrap();
    drop(file);

    let next_at = at + chrono::Duration::seconds(1);
    let third = collector
        .refresh_at(next_at)
        .await
        .unwrap();
    assert_eq!(collector.stats().events_applied, applied_after_first + 1);
    assert!(collector.stats().bytes_read > bytes_after_first);
    assert_eq!(third.sessions.len(), 1);
    assert_eq!(third.sessions[0].state, SessionState::Tool);

    collector
        .refresh_at(next_at)
        .await
        .unwrap();
    assert_eq!(collector.stats().events_applied, applied_after_first + 1);
}

#[tokio::test]
async fn duplicate_normalized_journal_uuid_is_applied_once() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let at = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
    let event = TelemetryEvent::new(
        ToolId::Codex,
        "journal-session",
        at,
        Confidence::Exact,
        EventKind::SessionStarted,
    );
    let encoded = serde_json::to_string(&event).unwrap();
    std::fs::write(
        data_dir.join("events.jsonl"),
        format!("{encoded}\n{encoded}\n"),
    )
    .unwrap();

    let mut config = passive_config(temp.path().join("empty-home"));
    config.home = None;
    let mut collector = LiveCollector::with_config(&data_dir, config);
    let snapshot = collector.refresh_at(at).await.unwrap();

    assert_eq!(collector.stats().events_seen, 2);
    assert_eq!(collector.stats().events_applied, 1);
    assert_eq!(collector.stats().duplicate_events, 1);
    assert_eq!(snapshot.sessions.len(), 1);
}
