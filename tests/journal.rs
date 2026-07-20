use chrono::{TimeZone, Utc};
use llmeter::journal::Journal;
use llmeter::model::{Confidence, EventKind, TelemetryEvent, ToolId};
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn journal_round_trip_skips_malformed_lines() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("events.jsonl");
    let journal = Journal::new(path.clone());
    let event = TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        Utc.with_ymd_and_hms(2026, 7, 20, 0, 0, 0).unwrap(),
        Confidence::Exact,
        EventKind::TurnStarted,
    );

    journal.append(&event).await.unwrap();
    tokio::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .await
        .unwrap()
        .write_all(b"not-json\n")
        .await
        .unwrap();

    let replayed = journal.read_all().await.unwrap();
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].session_id, "session-1");
}

#[tokio::test]
async fn snapshot_reorders_events_by_occurrence_time_before_aggregation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("out-of-order.jsonl");
    let journal = Journal::new(path.clone());
    let t0 = Utc.with_ymd_and_hms(2026, 7, 20, 0, 0, 0).unwrap();

    let output = TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        t0 + chrono::Duration::seconds(1),
        Confidence::Exact,
        EventKind::OutputDelta {
            tokens: Some(5),
            characters: 20,
            bytes: 20,
        },
    )
    .with_turn_id("turn-1");
    let start = TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        t0,
        Confidence::Exact,
        EventKind::TurnStarted,
    )
    .with_turn_id("turn-1");

    journal.append(&output).await.unwrap();
    journal.append(&start).await.unwrap();

    let snapshot = llmeter::runtime::snapshot_from_journal(&path)
        .await
        .unwrap();
    assert_eq!(snapshot.sessions[0].ttft_ms.value, Some(1_000.0));
    assert_eq!(snapshot.sessions[0].output_tokens, 5);
}
