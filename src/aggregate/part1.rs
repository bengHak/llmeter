#[derive(Clone, Debug)]
struct SessionRuntime {
    tool: ToolId,
    session_id: String,
    started_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    model: Option<String>,
    provider: Option<String>,
    cwd: Option<String>,
    pid: Option<u32>,
    current_turn: Option<TurnRuntime>,
    cumulative_input_tokens: u64,
    cumulative_output_tokens: u64,
    cumulative_cached_input_tokens: u64,
    cumulative_reasoning_tokens: u64,
    completed_input_tokens: u64,
    completed_output_tokens: u64,
    completed_cached_input_tokens: u64,
    completed_reasoning_tokens: u64,
    context_window: Option<u64>,
    exited_at: Option<DateTime<Utc>>,
    session_error: bool,
    seen_lifecycle: bool,
}

impl SessionRuntime {
    fn new(event: &TelemetryEvent) -> Self {
        Self {
            tool: event.tool,
            session_id: event.session_id.clone(),
            started_at: event.occurred_at,
            last_seen_at: event.occurred_at,
            model: None,
            provider: None,
            cwd: None,
            pid: None,
            current_turn: None,
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cached_input_tokens: 0,
            cumulative_reasoning_tokens: 0,
            completed_input_tokens: 0,
            completed_output_tokens: 0,
            completed_cached_input_tokens: 0,
            completed_reasoning_tokens: 0,
            context_window: None,
            exited_at: None,
            session_error: false,
            seen_lifecycle: false,
        }
    }

    fn begin_turn(&mut self, event: &TelemetryEvent) -> &mut TurnRuntime {
        let reuses_named_turn = event.turn_id.is_some()
            && self
                .current_turn
                .as_ref()
                .is_some_and(|turn| turn.id.as_ref() == event.turn_id.as_ref());
        if !reuses_named_turn {
            self.commit_current_turn();
            self.current_turn = Some(TurnRuntime::new(
                event.turn_id.clone(),
                event.occurred_at,
                event.confidence,
            ));
        }
        self.current_turn.as_mut().expect("turn was initialized")
    }

    fn ensure_turn(&mut self, event: &TelemetryEvent) -> &mut TurnRuntime {
        let needs_new = self.current_turn.as_ref().is_none_or(|turn| {
            turn.finished_at.is_some()
                || (event.turn_id.is_some() && turn.id.as_ref() != event.turn_id.as_ref())
        });
        if needs_new {
            self.commit_current_turn();
            self.current_turn = Some(TurnRuntime::new(
                event.turn_id.clone(),
                event.occurred_at,
                event.confidence,
            ));
        }
        self.current_turn.as_mut().expect("turn was initialized")
    }

    fn ensure_usage_turn(&mut self, event: &TelemetryEvent) -> &mut TurnRuntime {
        let needs_new = self.current_turn.as_ref().is_none_or(|turn| {
            event.turn_id.is_some() && turn.id.as_ref() != event.turn_id.as_ref()
        });
        if needs_new {
            self.commit_current_turn();
            self.current_turn = Some(TurnRuntime::new(
                event.turn_id.clone(),
                event.occurred_at,
                event.confidence,
            ));
        }
        self.current_turn.as_mut().expect("turn was initialized")
    }

    fn commit_current_turn(&mut self) {
        let Some(turn) = self.current_turn.take() else {
            return;
        };
        self.completed_input_tokens = self
            .completed_input_tokens
            .saturating_add(turn.reported_input_tokens);
        self.completed_output_tokens = self
            .completed_output_tokens
            .saturating_add(turn.accounted_output_tokens());
        self.completed_cached_input_tokens = self
            .completed_cached_input_tokens
            .saturating_add(turn.reported_cached_input_tokens);
        self.completed_reasoning_tokens = self
            .completed_reasoning_tokens
            .saturating_add(turn.reported_reasoning_tokens);
    }
}
