use std::fs::File;
use std::io::{BufRead, BufReader};

use chrono::{DateTime, Utc};
use llmeter::adapters::{adapter_for, AdapterContext};
use llmeter::model::{Confidence, EventKind, ToolId};

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
fn codex_calculates_rates_from_rollout_boundaries() {
    let mut adapter = adapter_for(ToolId::Codex);
    let observed_at = DateTime::parse_from_rfc3339("2026-07-20T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let records = [
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:00Z",
            "type": "event_msg",
            "payload": {"type": "task_started"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:02Z",
            "type": "response_item",
            "payload": {"type": "custom_tool_call", "call_id": "tool-1", "name": "shell"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:08Z",
            "type": "response_item",
            "payload": {"type": "custom_tool_call_output", "call_id": "tool-1"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:08Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {"output_tokens": 20},
                    "last_token_usage": {"output_tokens": 20}
                }
            }
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:10Z",
            "type": "response_item",
            "payload": {"type": "custom_tool_call", "call_id": "tool-2", "name": "shell"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:14Z",
            "type": "response_item",
            "payload": {"type": "custom_tool_call_output", "call_id": "tool-2"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:14Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {"output_tokens": 30},
                    "last_token_usage": {"output_tokens": 10}
                }
            }
        }),
    ];
    let events = records
        .iter()
        .flat_map(|record| {
            adapter.parse_record(record, &AdapterContext::new("codex-rate", observed_at))
        })
        .collect::<Vec<_>>();
    let rates = events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::RateReported {
                output_tokens,
                tokens_per_second,
            } => Some((output_tokens, tokens_per_second, event.confidence)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        rates,
        vec![
            (20, 10.0, Confidence::Derived),
            (10, 5.0, Confidence::Derived),
        ]
    );
}

#[test]
fn codex_counter_reset_restarts_rate_baseline() {
    let mut adapter = adapter_for(ToolId::Codex);
    let observed_at = DateTime::parse_from_rfc3339("2026-07-20T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let records = [
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:00Z",
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {"total_token_usage": {"output_tokens": 100}}}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:01Z",
            "type": "event_msg",
            "payload": {"type": "task_started"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:02Z",
            "type": "event_msg",
            "payload": {"type": "agent_message", "message": "private"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:02Z",
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {"total_token_usage": {"output_tokens": 10}}}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:03Z",
            "type": "event_msg",
            "payload": {"type": "task_started"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:05Z",
            "type": "event_msg",
            "payload": {"type": "agent_message", "message": "private"}
        }),
        serde_json::json!({
            "timestamp": "2026-07-20T00:00:05Z",
            "type": "event_msg",
            "payload": {"type": "token_count", "info": {"total_token_usage": {"output_tokens": 20}}}
        }),
    ];
    let events = records
        .iter()
        .flat_map(|record| {
            adapter.parse_record(record, &AdapterContext::new("codex-rate", observed_at))
        })
        .collect::<Vec<_>>();
    let rates = events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::RateReported {
                output_tokens,
                tokens_per_second,
            } => Some((output_tokens, tokens_per_second)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(rates, vec![(10, 5.0)]);
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
