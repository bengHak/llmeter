use std::fs::File;
use std::io::{BufRead, BufReader};

use chrono::{DateTime, Utc};
use llmeter::adapters::{adapter_for, AdapterContext};
use llmeter::model::{EventKind, ToolId};

fn parse_fixture(tool: ToolId, name: &str) -> Vec<llmeter::model::TelemetryEvent> {
    let file = File::open(format!("tests/fixtures/{name}.jsonl")).unwrap();
    let mut adapter = adapter_for(tool);
    let mut events = Vec::new();
    for line in BufReader::new(file).lines() {
        let value: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let observed_at = value
            .get("timestamp")
            .and_then(|value| value.as_str())
            .map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .unwrap()
                    .with_timezone(&Utc)
            })
            .unwrap();
        events.extend(adapter.parse_record(&value, &AdapterContext::new("fallback", observed_at)));
    }
    events
}

fn count(
    events: &[llmeter::model::TelemetryEvent],
    predicate: impl Fn(&EventKind) -> bool,
) -> usize {
    events.iter().filter(|event| predicate(&event.kind)).count()
}

#[test]
fn codex_exec_and_rollout_records_capture_usage_without_body() {
    let events = parse_fixture(ToolId::Codex, "codex");
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::TurnStarted)),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::OutputDelta { .. }
        )),
        1
    );
    assert!(
        count(&events, |kind| matches!(
            kind,
            EventKind::Usage {
                output_tokens: Some(25),
                ..
            }
        )) >= 1
    );
    assert!(
        count(&events, |kind| matches!(
            kind,
            EventKind::Usage {
                output_tokens: Some(30),
                context_window: Some(272000),
                ..
            }
        )) >= 1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::TurnFinished { .. }
        )),
        1
    );

    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains("private codex answer"));
}

#[test]
fn opencode_sse_events_capture_deltas_tools_and_idle_turn_end() {
    let events = parse_fixture(ToolId::OpenCode, "opencode");
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::SessionStarted)),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::TurnStarted)),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::OutputDelta { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::ToolStarted { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::ToolFinished { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::TurnFinished { .. }
        )),
        1
    );
}

#[test]
fn qwen_stream_json_captures_partial_output_usage_and_tool() {
    let events = parse_fixture(ToolId::Qwen, "qwen");
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::SessionStarted)),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::TurnStarted)),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::OutputDelta { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::ToolStarted { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::ToolFinished { .. }
        )),
        1
    );
    assert!(
        count(&events, |kind| matches!(
            kind,
            EventKind::Usage {
                output_tokens: Some(12),
                ..
            }
        )) >= 1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::TurnFinished { .. }
        )),
        1
    );

    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains("private qwen output"));
    assert!(!encoded.contains("private final answer"));
}

#[test]
fn kiro_acp_notifications_capture_stream_tool_and_turn_end() {
    let events = parse_fixture(ToolId::Kiro, "kiro");
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::SessionStarted)),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::TurnStarted)),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::OutputDelta { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::ToolStarted { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::ToolFinished { .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::TurnFinished { .. }
        )),
        1
    );

    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains("private prompt"));
    assert!(!encoded.contains("private output"));
}
