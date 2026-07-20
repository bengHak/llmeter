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
fn pi_rpc_stream_maps_output_and_tool_spans() {
    let events = parse_fixture(ToolId::Pi, "pi");
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
    assert!(!encoded.contains("hello world"));
    assert!(!encoded.contains("private command"));
    assert!(!encoded.contains("private output"));
}

#[test]
fn droid_hooks_and_stream_events_map_to_one_turn() {
    let events = parse_fixture(ToolId::Droid, "droid");
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
fn gemini_hooks_capture_chunk_usage_and_tools() {
    let events = parse_fixture(ToolId::Gemini, "gemini");
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
            EventKind::Usage {
                output_tokens: Some(3),
                ..
            }
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
    assert!(!encoded.contains("gemini output"));
    assert!(!encoded.contains("private final answer"));
}

#[test]
fn claude_hooks_capture_turn_tools_permission_and_session_end() {
    let events = parse_fixture(ToolId::Claude, "claude");
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::TurnStarted)),
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
            EventKind::WaitingForInput { .. }
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
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::SessionEnded)),
        1
    );

    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains("private prompt"));
    assert!(!encoded.contains("private answer"));
}
