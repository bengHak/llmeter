#[derive(Clone, Debug)]
pub struct Aggregator {
    config: AggregatorConfig,
    sessions: HashMap<SessionKey, SessionRuntime>,
}

impl Default for Aggregator {
    fn default() -> Self {
        Self::new(AggregatorConfig::default())
    }
}

impl Aggregator {
    pub fn new(config: AggregatorConfig) -> Self {
        Self {
            config,
            sessions: HashMap::new(),
        }
    }

    pub fn apply(&mut self, event: TelemetryEvent) {
        let key = SessionKey {
            tool: event.tool,
            session_id: event.session_id.clone(),
        };
        let session = self
            .sessions
            .entry(key)
            .or_insert_with(|| SessionRuntime::new(&event));

        session.last_seen_at = session.last_seen_at.max(event.occurred_at);

        match &event.kind {
            EventKind::SessionDiscovered { pid, cwd, model } => {
                session.pid = pid.or(session.pid);
                if let Some(cwd) = cwd {
                    session.cwd = Some(cwd.clone());
                }
                if let Some(model) = model {
                    session.model = Some(model.clone());
                }
            }
            EventKind::SessionStarted => {
                session.seen_lifecycle = true;
                session.exited_at = None;
            }
            EventKind::Metadata {
                cwd,
                model,
                provider,
                pid,
            } => {
                if let Some(cwd) = cwd {
                    session.cwd = Some(cwd.clone());
                }
                if let Some(model) = model {
                    session.model = Some(model.clone());
                }
                if let Some(provider) = provider {
                    session.provider = Some(provider.clone());
                }
                if let Some(pid) = pid {
                    session.pid = Some(*pid);
                }
            }
            EventKind::TurnStarted => {
                session.begin_turn(&event);
                session.session_error = false;
            }
            EventKind::FirstOutput => {
                let turn = session.ensure_turn(&event);
                if turn.first_output_at.is_none() {
                    turn.first_output_at = Some(event.occurred_at);
                    turn.first_output_confidence = event.confidence;
                    turn.last_output_at = Some(event.occurred_at);
                }
            }
            EventKind::OutputDelta {
                tokens, characters, ..
            } => {
                let turn = session.ensure_turn(&event);
                turn.record_output(event.occurred_at, *tokens, *characters, event.confidence);
            }
            EventKind::ToolStarted { call_id, .. } => {
                let turn = session.ensure_turn(&event);
                turn.waiting_for_input = false;
                if turn.active_tools.is_empty() {
                    turn.tool_block_started_at = Some(event.occurred_at);
                }
                turn.active_tools.insert(call_id.clone(), ActiveTool);
            }
            EventKind::ToolFinished { call_id, success } => {
                let turn = session.ensure_turn(&event);
                turn.finish_tool(call_id, event.occurred_at);
                if success == &Some(false) {
                    turn.failed = true;
                }
            }
            EventKind::Usage {
                input_tokens,
                output_tokens,
                cached_input_tokens,
                reasoning_tokens,
                context_window,
                cumulative,
            } => {
                if *cumulative {
                    update_cumulative_usage(&mut session.cumulative_input_tokens, *input_tokens);
                    update_cumulative_usage(&mut session.cumulative_output_tokens, *output_tokens);
                    update_cumulative_usage(
                        &mut session.cumulative_cached_input_tokens,
                        *cached_input_tokens,
                    );
                    update_cumulative_usage(
                        &mut session.cumulative_reasoning_tokens,
                        *reasoning_tokens,
                    );
                } else {
                    let turn = session.ensure_usage_turn(&event);
                    update_turn_usage(&mut turn.reported_input_tokens, *input_tokens);
                    update_turn_usage(&mut turn.reported_output_tokens, *output_tokens);
                    update_turn_usage(&mut turn.reported_cached_input_tokens, *cached_input_tokens);
                    update_turn_usage(&mut turn.reported_reasoning_tokens, *reasoning_tokens);
                }
                if let Some(context_window) = context_window {
                    session.context_window = Some(*context_window);
                }
            }
            EventKind::WaitingForInput { .. } => {
                let turn = session.ensure_turn(&event);
                turn.waiting_for_input = true;
            }
            EventKind::RetryStarted => {
                let turn = session.ensure_turn(&event);
                turn.retrying = true;
            }
            EventKind::RetryFinished { success } => {
                let turn = session.ensure_turn(&event);
                turn.retrying = false;
                if success == &Some(false) {
                    turn.failed = true;
                }
            }
            EventKind::TurnFinished { success } => {
                let turn = session.ensure_turn(&event);
                if !turn.active_tools.is_empty() {
                    turn.finish_tool_block(event.occurred_at);
                }
                turn.active_tools.clear();
                turn.finished_at = Some(event.occurred_at);
                turn.finish_confidence = event.confidence;
                turn.waiting_for_input = false;
                turn.retrying = false;
                turn.failed |= !success;
                session.session_error |= !success;
            }
            EventKind::Error { .. } => {
                session.session_error = true;
                if let Some(turn) = session.current_turn.as_mut() {
                    turn.failed = true;
                }
            }
            EventKind::SessionEnded => {
                session.exited_at = Some(event.occurred_at);
            }
            EventKind::Heartbeat => {}
        }
    }

    pub fn apply_all<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = TelemetryEvent>,
    {
        for event in events {
            self.apply(event);
        }
    }

    pub fn snapshot(&self, now: DateTime<Utc>) -> AppSnapshot {
        let mut snapshots = Vec::new();
        for session in self.sessions.values() {
            if let Some(exited_at) = session.exited_at {
                if now - exited_at > self.config.exited_session_ttl {
                    continue;
                }
            }
            snapshots.push(self.session_snapshot(session, now));
        }

        snapshots.sort_by(|left, right| {
            state_rank(left.state)
                .cmp(&state_rank(right.state))
                .then_with(|| rate_unit_rank(left.rate_unit).cmp(&rate_unit_rank(right.rate_unit)))
                .then_with(|| {
                    right
                        .current_tps
                        .value
                        .partial_cmp(&left.current_tps.value)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.tool.cmp(&right.tool))
                .then_with(|| left.session_id.cmp(&right.session_id))
        });

        let total_tps = snapshots
            .iter()
            .filter(|session| session.rate_unit == RateUnit::TokensPerSecond)
            .filter_map(|session| session.current_tps.value)
            .sum();
        let total_chars_per_second = snapshots
            .iter()
            .filter(|session| session.rate_unit == RateUnit::CharactersPerSecond)
            .filter_map(|session| session.current_tps.value)
            .sum();
        let active_sessions = snapshots
            .iter()
            .filter(|session| {
                matches!(
                    session.state,
                    SessionState::Queue
                        | SessionState::Stream
                        | SessionState::Tool
                        | SessionState::Input
                        | SessionState::Stall
                        | SessionState::Retry
                )
            })
            .count();
        let generating_sessions = snapshots
            .iter()
            .filter(|session| session.state == SessionState::Stream)
            .count();
        let stalled_sessions = snapshots
            .iter()
            .filter(|session| session.state == SessionState::Stall)
            .count();
        let error_sessions = snapshots
            .iter()
            .filter(|session| session.state == SessionState::Error)
            .count();

        AppSnapshot {
            generated_at: now,
            sessions: snapshots,
            total_tps,
            total_chars_per_second,
            active_sessions,
            generating_sessions,
            stalled_sessions,
            error_sessions,
        }
    }

    fn session_snapshot(&self, session: &SessionRuntime, now: DateTime<Utc>) -> SessionSnapshot {
        let turn = session.current_turn.as_ref();
        let state = derive_state(session, now, self.config.stall_threshold);
        let current_rate = turn.and_then(|turn| recent_rate(turn, now, self.config.rate_window));
        let average = turn.and_then(|turn| average_rate(turn, now));
        let current_tps = current_rate
            .map(|(metric, _)| metric)
            .unwrap_or_else(MetricValue::unknown);
        let turn_average_tps = average
            .map(|(metric, _)| metric)
            .unwrap_or_else(MetricValue::unknown);
        let rate_unit = current_rate
            .map(|(_, unit)| unit)
            .or_else(|| average.map(|(_, unit)| unit))
            .unwrap_or_default();
        let ttft_ms = turn
            .and_then(|turn| {
                turn.first_output_at.map(|first| {
                    MetricValue::new(
                        non_negative_duration(first - turn.started_at).num_milliseconds() as f64,
                        turn.start_confidence.lower(turn.first_output_confidence),
                    )
                })
            })
            .unwrap_or_else(MetricValue::unknown);
        let e2e_ms = turn
            .map(|turn| {
                let end = turn.finished_at.unwrap_or(now);
                let confidence = if turn.finished_at.is_some() {
                    turn.start_confidence.lower(turn.finish_confidence)
                } else {
                    Confidence::Derived.lower(turn.start_confidence)
                };
                MetricValue::new(
                    non_negative_duration(end - turn.started_at).num_milliseconds() as f64,
                    confidence,
                )
            })
            .unwrap_or_else(MetricValue::unknown);

        let current_input_tokens = turn.map_or(0, |turn| turn.reported_input_tokens);
        let current_output_tokens = turn.map_or(0, TurnRuntime::accounted_output_tokens);
        let current_cached_input_tokens = turn.map_or(0, |turn| turn.reported_cached_input_tokens);
        let current_reasoning_tokens = turn.map_or(0, |turn| turn.reported_reasoning_tokens);
        let input_tokens = session.cumulative_input_tokens.max(
            session
                .completed_input_tokens
                .saturating_add(current_input_tokens),
        );
        let output_tokens = session.cumulative_output_tokens.max(
            session
                .completed_output_tokens
                .saturating_add(current_output_tokens),
        );
        let cached_input_tokens = session.cumulative_cached_input_tokens.max(
            session
                .completed_cached_input_tokens
                .saturating_add(current_cached_input_tokens),
        );
        let reasoning_tokens = session.cumulative_reasoning_tokens.max(
            session
                .completed_reasoning_tokens
                .saturating_add(current_reasoning_tokens),
        );
        let (tool_wait, stall) = turn.map_or((Duration::zero(), Duration::zero()), |turn| {
            (
                current_tool_wait(turn, now),
                current_stall(turn, now, self.config.stall_threshold),
            )
        });

        SessionSnapshot {
            tool: session.tool,
            session_id: session.session_id.clone(),
            state,
            turn_id: turn.and_then(|turn| turn.id.clone()),
            model: session.model.clone(),
            provider: session.provider.clone(),
            cwd: session.cwd.clone(),
            pid: session.pid,
            started_at: session.started_at,
            last_seen_at: session.last_seen_at,
            current_tps,
            turn_average_tps,
            rate_unit,
            ttft_ms,
            e2e_ms,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            reasoning_tokens,
            context_window: session.context_window,
            tool_wait_ms: tool_wait.num_milliseconds(),
            stall_ms: stall.num_milliseconds(),
        }
    }
}
