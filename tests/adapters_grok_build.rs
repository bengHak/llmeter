use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use llmeter::adapters::{adapter_for, AdapterContext};
use llmeter::model::{EventKind, ToolId};

fn at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn parse_fixture() -> Vec<llmeter::model::TelemetryEvent> {
    let file = File::open("tests/fixtures/grok_build.jsonl").unwrap();
    let mut adapter = adapter_for(ToolId::GrokBuild);
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
            .unwrap_or_else(|| Utc.timestamp_opt(1_784_505_900, 0).unwrap());
        events.extend(adapter.parse_record(
            &value,
            &AdapterContext::new("grok-headless-process", observed_at),
        ));
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
fn grok_build_fixture_normalizes_hooks_headless_acp_and_persisted_updates() {
    let events = parse_fixture();

    assert_eq!(count(&events, |kind| matches!(kind, EventKind::TurnStarted)), 4);
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::OutputDelta { .. })),
        5
    );
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::ToolStarted { .. })),
        3
    );
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::ToolFinished { .. })),
        3
    );
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::TurnFinished { .. })),
        4
    );
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::WaitingForInput { .. })),
        1
    );
    assert_eq!(count(&events, |kind| matches!(kind, EventKind::RetryStarted)), 1);
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::RetryFinished { success: Some(true) })),
        1
    );

    let usage: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Usage {
                input_tokens,
                output_tokens,
                cached_input_tokens,
                reasoning_tokens,
                cumulative,
                ..
            } => Some((
                *input_tokens,
                *output_tokens,
                *cached_input_tokens,
                *reasoning_tokens,
                *cumulative,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        usage,
        vec![
            (Some(100), Some(12), Some(40), Some(4), false),
            (Some(180), Some(20), Some(60), Some(6), false),
            (Some(240), Some(32), Some(80), Some(9), false),
        ]
    );
}

#[test]
fn grok_build_normalized_events_do_not_retain_private_payloads() {
    let encoded = serde_json::to_string(&parse_fixture()).unwrap();
    for private in [
        "private hook prompt",
        "private command",
        "private tool result",
        "private hook answer",
        "private headless answer",
        "private headless thought",
        "private acp prompt",
        "private acp thought",
        "private acp answer",
        "private acp command",
        "private acp output",
        "private persisted prompt",
        "private retry reason",
        "private persisted answer",
        "private persisted command",
        "private persisted output",
        "private persisted final",
    ] {
        assert!(!encoded.contains(private), "leaked {private}");
    }
}

#[test]
fn grok_build_passive_updates_use_the_parent_session_directory() {
    let mut adapter = adapter_for(ToolId::GrokBuild);
    let value = serde_json::json!({
        "timestamp": 1784505900_i64,
        "method": "session/update",
        "params": {
            "update": {
                "sessionUpdate": "user_message_chunk",
                "content": {"type": "text", "text": "private prompt"}
            }
        }
    });
    let context = AdapterContext::new("updates", Utc.timestamp_opt(1_784_505_900, 0).unwrap())
        .with_source_path(PathBuf::from(
            "/home/user/.grok/sessions/project/session-42/updates.jsonl",
        ));
    let events = adapter.parse_record(&value, &context);

    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event.session_id == "session-42"));
}

#[test]
fn grok_build_failure_hooks_close_tool_and_turn_without_raw_error_text() {
    let mut adapter = adapter_for(ToolId::GrokBuild);
    let observed_at = Utc.timestamp_opt(1_784_505_900, 0).unwrap();
    let records = [
        serde_json::json!({
            "hookEventName": "UserPromptSubmit",
            "sessionId": "grok-failure",
            "prompt": "private prompt"
        }),
        serde_json::json!({
            "hookEventName": "PreToolUse",
            "sessionId": "grok-failure",
            "toolName": "run_terminal_command",
            "toolUseId": "tool-failure",
            "toolInput": {"command": "private command"}
        }),
        serde_json::json!({
            "hookEventName": "PermissionDenied",
            "sessionId": "grok-failure",
            "toolName": "run_terminal_command",
            "toolUseId": "tool-failure",
            "reason": "private permission reason"
        }),
        serde_json::json!({
            "hookEventName": "StopFailure",
            "sessionId": "grok-failure",
            "error": "private stop failure"
        }),
    ];
    let mut events = Vec::new();
    for value in records {
        events.extend(adapter.parse_record(
            &value,
            &AdapterContext::new("fallback", observed_at),
        ));
    }

    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::ToolFinished { success: Some(false), .. }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::TurnFinished { success: false }
        )),
        1
    );
    assert_eq!(count(&events, |kind| matches!(kind, EventKind::Error { .. })), 2);
    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains("private permission reason"));
    assert!(!encoded.contains("private stop failure"));
}

#[test]
fn grok_build_single_json_result_uses_wrapper_session_and_reports_usage() {
    let mut adapter = adapter_for(ToolId::GrokBuild);
    let observed_at = Utc.timestamp_opt(1_784_505_900, 0).unwrap();
    let value = serde_json::json!({
        "text": "private single json answer",
        "stopReason": "end_turn",
        "sessionId": "native-session-visible-only-at-end",
        "usage": {
            "inputTokens": 55,
            "outputTokens": 8,
            "cachedReadTokens": 10,
            "reasoningTokens": 2
        },
        "modelUsage": {
            "grok-4.5": {
                "inputTokens": 55,
                "outputTokens": 8,
                "cacheReadInputTokens": 10
            }
        }
    });
    let events = adapter.parse_record(
        &value,
        &AdapterContext::new("grok-process-99", observed_at),
    );

    assert!(events
        .iter()
        .all(|event| event.session_id == "grok-process-99"));
    assert_eq!(
        count(&events, |kind| matches!(kind, EventKind::OutputDelta { .. })),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::Usage {
                input_tokens: Some(55),
                output_tokens: Some(8),
                cached_input_tokens: Some(10),
                reasoning_tokens: Some(2),
                cumulative: false,
                ..
            }
        )),
        1
    );
    assert_eq!(
        count(&events, |kind| matches!(
            kind,
            EventKind::TurnFinished { success: true }
        )),
        1
    );
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("private single json answer"));
}

#[test]
fn grok_unified_metrics_fold_child_inference_into_the_pid_root() {
    let mut adapter = adapter_for(ToolId::GrokBuild);
    let observed_at = Utc.timestamp_opt(1_784_505_900, 0).unwrap();
    let records = [
        serde_json::json!({
            "ts": "2026-07-21T14:30:56.958Z",
            "pid": 93524,
            "sid": "root-session",
            "src": "shell",
            "msg": "session created",
            "ctx": {"cwd": "/work/AquaTick"}
        }),
        serde_json::json!({
            "ts": "2026-07-21T14:31:09.280Z",
            "pid": 93524,
            "sid": "child-session",
            "src": "shell",
            "msg": "shell.turn.inference_done",
            "ctx": {
                "tokens_per_sec": 60.0,
                "completion_tokens": 12,
                "prompt_tokens": 100,
                "cached_prompt_tokens": 40,
                "reasoning_tokens": 4
            }
        }),
        serde_json::json!({
            "ts": "2026-07-21T14:31:15.782Z",
            "pid": 93524,
            "sid": "root-session",
            "src": "shell",
            "msg": "shell.turn.inference_done",
            "ctx": {
                "tokens_per_sec": 100.0,
                "completion_tokens": 20,
                "prompt_tokens": 180,
                "cached_prompt_tokens": 60,
                "reasoning_tokens": 6
            }
        }),
    ];
    let events = records
        .iter()
        .flat_map(|record| {
            adapter.parse_record(record, &AdapterContext::new("unified", observed_at))
        })
        .collect::<Vec<_>>();

    assert!(events
        .iter()
        .all(|event| event.session_id == "root-session"));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::Metadata { cwd: Some(cwd), .. } if cwd == "/work/AquaTick"
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::RateReported { .. }))
            .count(),
        2
    );
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::Usage {
            input_tokens: Some(280),
            output_tokens: Some(32),
            cached_input_tokens: Some(100),
            reasoning_tokens: Some(10),
            cumulative: true,
            ..
        }
    )));
    assert_eq!(
        events.last().unwrap().occurred_at,
        at("2026-07-21T14:31:15.782Z")
    );
}
