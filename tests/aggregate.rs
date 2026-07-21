use chrono::{DateTime, Duration, Utc};
use llmeter::aggregate::{Aggregator, AggregatorConfig};
use llmeter::model::{Confidence, EventKind, RateUnit, SessionState, TelemetryEvent, ToolId};

fn at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn event(time: DateTime<Utc>, kind: EventKind) -> TelemetryEvent {
    TelemetryEvent::new(ToolId::Pi, "session-1", time, Confidence::Exact, kind)
        .with_turn_id("turn-1")
}

#[test]
fn empty_snapshot_uses_positive_zero_throughput() {
    let snapshot = Aggregator::default().snapshot(at("2026-07-20T00:00:00Z"));

    assert_eq!(snapshot.total_tps, 0.0);
    assert!(snapshot.total_tps.is_sign_positive());
}

#[test]
fn calculates_ttft_recent_rate_and_turn_average() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::new(AggregatorConfig::default());

    aggregate.apply(event(t0, EventKind::TurnStarted));
    aggregate.apply(event(
        t0 + Duration::seconds(1),
        EventKind::OutputDelta {
            tokens: Some(20),
            characters: 80,
            bytes: 80,
        },
    ));
    aggregate.apply(event(
        t0 + Duration::seconds(2),
        EventKind::OutputDelta {
            tokens: Some(10),
            characters: 40,
            bytes: 40,
        },
    ));

    let snapshot = aggregate.snapshot(t0 + Duration::seconds(2));
    let session = &snapshot.sessions[0];

    assert_eq!(session.ttft_ms.value, Some(1_000.0));
    assert_eq!(session.ttft_ms.confidence, Confidence::Exact);
    assert_eq!(session.current_tps.value, Some(30.0));
    assert_eq!(session.turn_average_tps.value, Some(30.0));
    assert_eq!(session.output_tokens, 30);
    assert_eq!(session.state, SessionState::Stream);
}

#[test]
fn tool_state_precedes_stall_and_stall_resumes_after_tool_finishes() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::new(AggregatorConfig {
        stall_threshold: Duration::seconds(2),
        ..AggregatorConfig::default()
    });

    aggregate.apply(event(t0, EventKind::TurnStarted));
    aggregate.apply(event(
        t0 + Duration::milliseconds(100),
        EventKind::OutputDelta {
            tokens: Some(1),
            characters: 4,
            bytes: 4,
        },
    ));
    aggregate.apply(event(
        t0 + Duration::seconds(1),
        EventKind::ToolStarted {
            call_id: "tool-1".into(),
            name: "bash".into(),
        },
    ));

    let during_tool = aggregate.snapshot(t0 + Duration::seconds(4));
    assert_eq!(during_tool.sessions[0].state, SessionState::Tool);

    aggregate.apply(event(
        t0 + Duration::seconds(5),
        EventKind::ToolFinished {
            call_id: "tool-1".into(),
            success: Some(true),
        },
    ));
    let stalled = aggregate.snapshot(t0 + Duration::milliseconds(6_200));
    assert_eq!(stalled.sessions[0].state, SessionState::Stall);
    assert_eq!(stalled.sessions[0].tool_wait_ms, 4_000);
}

#[test]
fn overlapping_tool_spans_count_wall_clock_wait_once() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::default();

    aggregate.apply(event(t0, EventKind::TurnStarted));
    aggregate.apply(event(
        t0 + Duration::seconds(1),
        EventKind::ToolStarted {
            call_id: "tool-1".into(),
            name: "first".into(),
        },
    ));
    aggregate.apply(event(
        t0 + Duration::seconds(2),
        EventKind::ToolStarted {
            call_id: "tool-2".into(),
            name: "second".into(),
        },
    ));
    aggregate.apply(event(
        t0 + Duration::seconds(4),
        EventKind::ToolFinished {
            call_id: "tool-1".into(),
            success: Some(true),
        },
    ));
    aggregate.apply(event(
        t0 + Duration::seconds(5),
        EventKind::ToolFinished {
            call_id: "tool-2".into(),
            success: Some(true),
        },
    ));

    let snapshot = aggregate.snapshot(t0 + Duration::seconds(5));
    assert_eq!(snapshot.sessions[0].tool_wait_ms, 4_000);
}

#[test]
fn cumulative_usage_replaces_instead_of_double_counting() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::default();

    for (offset, output) in [(1, 10), (2, 25)] {
        aggregate.apply(event(
            t0 + Duration::seconds(offset),
            EventKind::Usage {
                input_tokens: Some(100),
                output_tokens: Some(output),
                cached_input_tokens: Some(20),
                reasoning_tokens: None,
                context_window: Some(200_000),
                cumulative: true,
            },
        ));
    }

    let snapshot = aggregate.snapshot(t0 + Duration::seconds(2));
    let session = &snapshot.sessions[0];
    assert_eq!(session.input_tokens, 100);
    assert_eq!(session.output_tokens, 25);
    assert_eq!(session.cached_input_tokens, 20);
    assert_eq!(session.context_window, Some(200_000));
}

#[test]
fn repeated_turn_usage_is_not_double_counted_and_new_turns_accumulate() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::default();

    aggregate.apply(event(t0, EventKind::TurnStarted));
    for offset in [1, 2] {
        aggregate.apply(event(
            t0 + Duration::seconds(offset),
            EventKind::Usage {
                input_tokens: Some(100),
                output_tokens: Some(12),
                cached_input_tokens: Some(20),
                reasoning_tokens: None,
                context_window: None,
                cumulative: false,
            },
        ));
    }
    aggregate.apply(event(
        t0 + Duration::seconds(3),
        EventKind::TurnFinished { success: true },
    ));

    let second_turn = TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        t0 + Duration::seconds(4),
        Confidence::Exact,
        EventKind::TurnStarted,
    )
    .with_turn_id("turn-2");
    aggregate.apply(second_turn);
    let second_usage = TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        t0 + Duration::seconds(5),
        Confidence::Exact,
        EventKind::Usage {
            input_tokens: Some(50),
            output_tokens: Some(8),
            cached_input_tokens: Some(5),
            reasoning_tokens: None,
            context_window: None,
            cumulative: false,
        },
    )
    .with_turn_id("turn-2");
    aggregate.apply(second_usage);

    let snapshot = aggregate.snapshot(t0 + Duration::seconds(5));
    let session = &snapshot.sessions[0];
    assert_eq!(session.input_tokens, 150);
    assert_eq!(session.output_tokens, 20);
    assert_eq!(session.cached_input_tokens, 25);
}

#[test]
fn mixed_stream_deltas_and_turn_usage_accumulate_without_double_counting() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::default();

    aggregate.apply(event(t0, EventKind::TurnStarted));
    aggregate.apply(event(
        t0 + Duration::seconds(1),
        EventKind::OutputDelta {
            tokens: Some(10),
            characters: 40,
            bytes: 40,
        },
    ));
    aggregate.apply(event(
        t0 + Duration::seconds(2),
        EventKind::TurnFinished { success: true },
    ));

    let second_turn = TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        t0 + Duration::seconds(3),
        Confidence::Exact,
        EventKind::TurnStarted,
    )
    .with_turn_id("turn-2");
    aggregate.apply(second_turn);
    let second_usage = TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        t0 + Duration::seconds(4),
        Confidence::Exact,
        EventKind::Usage {
            input_tokens: Some(30),
            output_tokens: Some(7),
            cached_input_tokens: None,
            reasoning_tokens: None,
            context_window: None,
            cumulative: false,
        },
    )
    .with_turn_id("turn-2");
    aggregate.apply(second_usage);

    let mixed_snapshot = aggregate.snapshot(t0 + Duration::seconds(4));
    assert_eq!(mixed_snapshot.sessions[0].output_tokens, 17);

    aggregate.apply(TelemetryEvent::new(
        ToolId::Pi,
        "session-1",
        t0 + Duration::seconds(5),
        Confidence::Exact,
        EventKind::Usage {
            input_tokens: Some(130),
            output_tokens: Some(20),
            cached_input_tokens: None,
            reasoning_tokens: None,
            context_window: None,
            cumulative: true,
        },
    ));
    let cumulative_snapshot = aggregate.snapshot(t0 + Duration::seconds(5));
    assert_eq!(cumulative_snapshot.sessions[0].input_tokens, 130);
    assert_eq!(cumulative_snapshot.sessions[0].output_tokens, 20);
}

#[test]
fn character_only_deltas_do_not_produce_token_rates() {
    let t0 = at("2026-07-20T00:00:00Z");
    let mut aggregate = Aggregator::default();

    aggregate.apply(event(t0, EventKind::TurnStarted));
    aggregate.apply(event(
        t0 + Duration::seconds(1),
        EventKind::OutputDelta {
            tokens: None,
            characters: 40,
            bytes: 40,
        },
    ));

    let snapshot = aggregate.snapshot(t0 + Duration::seconds(2));
    let session = &snapshot.sessions[0];
    // Timing still advances (TTFT), but rates stay unknown without token counts.
    assert_eq!(session.ttft_ms.value, Some(1_000.0));
    assert_eq!(session.rate_unit, RateUnit::Unknown);
    assert_eq!(session.current_tps.value, None);
    assert_eq!(session.turn_average_tps.value, None);
    assert_eq!(snapshot.total_tps, 0.0);
}

#[test]
fn normalized_event_serialization_never_contains_source_text() {
    let secret = "DO-NOT-PERSIST-this prompt or response";
    let event = TelemetryEvent::new(
        ToolId::Claude,
        "privacy-session",
        at("2026-07-20T00:00:00Z"),
        Confidence::Estimated,
        EventKind::OutputDelta {
            tokens: None,
            characters: secret.chars().count() as u64,
            bytes: secret.len() as u64,
        },
    );

    let encoded = serde_json::to_string(&event).unwrap();
    assert!(!encoded.contains(secret));
    assert!(!encoded.contains("DO-NOT-PERSIST"));
    assert!(encoded.contains("characters"));
}
