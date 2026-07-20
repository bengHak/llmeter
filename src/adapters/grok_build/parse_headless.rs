impl GrokBuildAdapter {
    fn parse_headless(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let session = context.fallback_session_id.clone();
        let record_type = string_at(value, &["type"])
            .map(normalize_name)
            .unwrap_or_else(|| "result".to_owned());

        match record_type.as_str() {
            "text" | "thought" => {
                if let Some(text) = first_string_at(value, &[&["data"], &["text"]]) {
                    push_output(
                        &mut output,
                        &session,
                        text,
                        value,
                        context,
                        &mut self.state,
                        Confidence::Estimated,
                    );
                }
            }
            "end" => {
                push_usage(&mut output, &session, value, context, &self.state);
                push_metadata(&mut output, &session, value, context, model_from_usage(value));
                let success = !matches!(
                    first_string_at(value, &[&["stopReason"], &["stop_reason"]])
                        .map(normalize_name)
                        .as_deref(),
                    Some("error" | "failed" | "maxturnsreached")
                );
                finish_turn(
                    &mut output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    success,
                    Confidence::Exact,
                );
            }
            "error" | "maxturnsreached" => {
                let turn = ensure_turn_started(
                    &mut output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    Confidence::Derived,
                );
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::Error {
                        code: Some(if record_type == "maxturnsreached" {
                            "max_turns_reached"
                        } else {
                            "grok_headless_error"
                        }
                        .to_owned()),
                    },
                ));
                finish_turn(
                    &mut output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    false,
                    Confidence::Exact,
                );
            }
            "result" => {
                if let Some(text) = string_at(value, &["text"]) {
                    push_output(
                        &mut output,
                        &session,
                        text,
                        value,
                        context,
                        &mut self.state,
                        Confidence::Estimated,
                    );
                }
                push_usage(&mut output, &session, value, context, &self.state);
                push_metadata(&mut output, &session, value, context, model_from_usage(value));
                let success = value.get("error").is_none()
                    && !matches!(
                        first_string_at(value, &[&["stopReason"], &["stop_reason"]])
                            .map(normalize_name)
                            .as_deref(),
                        Some("error" | "failed" | "maxturnsreached")
                    );
                if !success {
                    let turn = self.state.current_turn(&session).or_else(|| turn_id(value));
                    output.push(event(
                        self.tool(),
                        &session,
                        turn.as_deref(),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::Error {
                            code: Some("grok_headless_error".to_owned()),
                        },
                    ));
                }
                finish_turn(
                    &mut output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    success,
                    Confidence::Exact,
                );
            }
            _ => {}
        }

        output
    }
}
