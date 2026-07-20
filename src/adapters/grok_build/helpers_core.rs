fn hook_name(value: &Value) -> Option<&str> {
    first_string_at(value, &[&["hookEventName"], &["hook_event_name"]])
}

fn is_headless_record(value: &Value) -> bool {
    if let Some(kind) = string_at(value, &["type"]) {
        return matches!(
            normalize_name(kind).as_str(),
            "text" | "thought" | "end" | "error" | "maxturnsreached"
        );
    }
    value.get("text").is_some()
        && (value.get("stopReason").is_some()
            || value.get("stop_reason").is_some()
            || value.get("usage").is_some()
            || value.get("sessionId").is_some())
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn effective_session_id(value: &Value, context: &AdapterContext) -> String {
    if let Some(session) = first_string_at(
        value,
        &[
            &["sessionId"],
            &["session_id"],
            &["params", "sessionId"],
            &["params", "session_id"],
            &["result", "sessionId"],
            &["result", "session_id"],
        ],
    ) {
        return session.to_owned();
    }
    if let Some(path) = context.source_path.as_deref() {
        if path.file_name().and_then(|name| name.to_str()) == Some("updates.jsonl") {
            if let Some(session) = path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
            {
                return session.to_owned();
            }
        }
    }
    context.fallback_session_id.clone()
}

fn request_id(value: &Value) -> Option<String> {
    match value.get("id")? {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn grok_tool_call_id(value: &Value) -> String {
    first_string_at(
        value,
        &[
            &["toolUseId"],
            &["tool_use_id"],
            &["toolCallId"],
            &["tool_call_id"],
            &["params", "update", "toolCallId"],
            &["params", "update", "tool_call_id"],
        ],
    )
    .unwrap_or("grok-tool")
    .to_owned()
}

fn grok_tool_name(value: &Value) -> String {
    first_string_at(
        value,
        &[
            &["toolName"],
            &["tool_name"],
            &["params", "update", "title"],
            &["params", "update", "toolName"],
            &["params", "update", "tool_name"],
        ],
    )
    .unwrap_or("tool")
    .to_owned()
}

fn is_finished_status(status: &str) -> bool {
    matches!(status, "completed" | "success" | "failed" | "error" | "cancelled")
}

fn ensure_turn_started(
    output: &mut Vec<TelemetryEvent>,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
    confidence: Confidence,
) -> String {
    if let Some(turn) = state.current_turn(session) {
        return turn;
    }
    let turn = state.begin_turn(session, turn_id(value));
    output.push(event(
        ToolId::GrokBuild,
        session,
        Some(&turn),
        value,
        context,
        confidence,
        EventKind::TurnStarted,
    ));
    turn
}

fn push_output(
    output: &mut Vec<TelemetryEvent>,
    session: &str,
    text: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
    confidence: Confidence,
) {
    if text.is_empty() {
        return;
    }
    let turn = ensure_turn_started(output, session, value, context, state, confidence);
    state.mark_output(session);
    output.push(event(
        ToolId::GrokBuild,
        session,
        Some(&turn),
        value,
        context,
        confidence,
        safe_output_delta(text),
     ));
}

fn finish_tool(
    output: &mut Vec<TelemetryEvent>,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
    success: bool,
) {
    let turn = ensure_turn_started(
        output,
        session,
        value,
        context,
        state,
        Confidence::Exact,
    );
    output.push(event(
        ToolId::GrokBuild,
        session,
        Some(&turn),
        value,
        context,
        Confidence::Exact,
        EventKind::ToolFinished {
            call_id: grok_tool_call_id(value),
            success: Some(success),
        },
    ));
}

fn finish_turn(
    output: &mut Vec<TelemetryEvent>,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
    success: bool,
    confidence: Confidence,
) {
    let Some(turn) = state.current_turn(session).or_else(|| turn_id(value)) else {
        return;
    };
    output.push(event(
        ToolId::GrokBuild,
        session,
        Some(&turn),
        value,
        context,
        confidence,
        EventKind::TurnFinished { success },
    ));
    state.end_turn(session);
}

