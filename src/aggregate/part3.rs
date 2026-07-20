fn update_cumulative_usage(target: &mut u64, value: Option<u64>) {
    let Some(value) = value else {
        return;
    };
    *target = (*target).max(value);
}

fn update_turn_usage(target: &mut u64, value: Option<u64>) {
    let Some(value) = value else {
        return;
    };
    *target = (*target).max(value);
}

fn recent_rate(
    turn: &TurnRuntime,
    now: DateTime<Utc>,
    window: Duration,
) -> Option<(MetricValue, RateUnit)> {
    if turn.samples.is_empty() {
        return None;
    }
    let lower_bound = now - window;
    let within_window: Vec<_> = turn
        .samples
        .iter()
        .filter(|sample| sample.at >= lower_bound && sample.at <= now)
        .collect();
    if within_window.is_empty() {
        return Some((
            MetricValue::new(0.0, turn.token_output_confidence),
            RateUnit::TokensPerSecond,
        ));
    }

    let units: f64 = within_window.iter().map(|sample| sample.units).sum();
    let confidence = within_window
        .iter()
        .map(|sample| sample.confidence)
        .reduce(Confidence::lower)
        .unwrap_or(Confidence::Unknown);
    let start = within_window
        .first()
        .map(|sample| sample.at)
        .unwrap_or(lower_bound)
        .max(lower_bound);
    let elapsed = non_negative_duration(now - start);
    let seconds = (elapsed.num_milliseconds().max(100) as f64) / 1_000.0;
    Some((
        MetricValue::new(units / seconds, confidence),
        RateUnit::TokensPerSecond,
    ))
}

fn average_rate(turn: &TurnRuntime, now: DateTime<Utc>) -> Option<(MetricValue, RateUnit)> {
    if turn.token_output_units <= 0.0 {
        return None;
    }
    let first = turn.samples.front().map(|sample| sample.at)?;
    let end = turn
        .finished_at
        .or(turn.last_output_at)
        .unwrap_or(now)
        .max(first);
    let elapsed = non_negative_duration(end - first);
    let seconds = (elapsed.num_milliseconds().max(100) as f64) / 1_000.0;
    Some((
        MetricValue::new(
            turn.token_output_units / seconds,
            turn.token_output_confidence,
        ),
        RateUnit::TokensPerSecond,
    ))
}

fn lower_known_confidence(left: Confidence, right: Confidence) -> Confidence {
    if left == Confidence::Unknown {
        right
    } else {
        left.lower(right)
    }
}

fn derive_state(
    session: &SessionRuntime,
    now: DateTime<Utc>,
    stall_threshold: Duration,
) -> SessionState {
    if session.exited_at.is_some() {
        return SessionState::Exited;
    }
    if session.session_error {
        return SessionState::Error;
    }
    let Some(turn) = session.current_turn.as_ref() else {
        return if session.seen_lifecycle {
            SessionState::Idle
        } else {
            SessionState::New
        };
    };
    if turn.finished_at.is_some() {
        return if turn.failed {
            SessionState::Error
        } else {
            SessionState::Idle
        };
    }
    if turn.retrying {
        return SessionState::Retry;
    }
    if !turn.active_tools.is_empty() {
        return SessionState::Tool;
    }
    if turn.waiting_for_input {
        return SessionState::Input;
    }
    if current_stall(turn, now, stall_threshold) > Duration::zero() {
        return SessionState::Stall;
    }
    if turn.first_output_at.is_some() {
        return SessionState::Stream;
    }
    SessionState::Queue
}

fn current_tool_wait(turn: &TurnRuntime, now: DateTime<Utc>) -> Duration {
    turn.tool_wait
        + turn
            .tool_block_started_at
            .map_or(Duration::zero(), |started_at| {
                non_negative_duration(now - started_at)
            })
}

fn current_stall(turn: &TurnRuntime, now: DateTime<Utc>, threshold: Duration) -> Duration {
    if turn.finished_at.is_some()
        || turn.waiting_for_input
        || turn.retrying
        || !turn.active_tools.is_empty()
    {
        return Duration::zero();
    }
    let Some(last_output) = turn.last_output_at else {
        return Duration::zero();
    };
    let effective_silence =
        non_negative_duration(now - last_output) - turn.tool_wait_since_last_output;
    if effective_silence >= threshold {
        effective_silence
    } else {
        Duration::zero()
    }
}

fn non_negative_duration(duration: Duration) -> Duration {
    if duration < Duration::zero() {
        Duration::zero()
    } else {
        duration
    }
}

fn state_rank(state: SessionState) -> u8 {
    match state {
        SessionState::Error => 0,
        SessionState::Stall => 1,
        SessionState::Retry => 2,
        SessionState::Tool => 3,
        SessionState::Stream => 4,
        SessionState::Queue => 5,
        SessionState::Input => 6,
        SessionState::New => 7,
        SessionState::Idle => 8,
        SessionState::Exited => 9,
        SessionState::Unknown => 10,
    }
}

fn rate_unit_rank(unit: RateUnit) -> u8 {
    match unit {
        RateUnit::TokensPerSecond => 0,
        RateUnit::Unknown => 1,
    }
}
