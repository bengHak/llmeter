impl GrokBuildAdapter {
    fn parse_rpc_response(
        &mut self,
        output: &mut Vec<TelemetryEvent>,
        value: &Value,
        context: &AdapterContext,
    ) {
        let Some(id) = request_id(value) else {
            return;
        };
        if let Some(native_session) = first_string_at(
            value,
            &[&["result", "sessionId"], &["result", "session_id"]],
        ) {
            self.pending_new_sessions.remove(&id);
            output.push(event(
                self.tool(),
                native_session,
                None,
                value,
                context,
                Confidence::Exact,
                EventKind::SessionStarted,
            ));
            push_metadata(output, native_session, value, context, None);
            return;
        }
        if let Some(session) = self.pending_new_sessions.remove(&id) {
            if value.get("error").is_none() {
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Derived,
                    EventKind::SessionStarted,
                ));
                push_metadata(output, &session, value, context, None);
            }
            return;
        }

        let Some(session) = self.pending_prompts.remove(&id) else {
            return;
        };
        push_usage(output, &session, value, context, &self.state);
        push_metadata(output, &session, value, context, model_from_usage(value));
        let success = value.get("error").is_none();
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
                    code: Some("grok_prompt_error".to_owned()),
                },
            ));
        }
        finish_turn(
            output,
            &session,
            value,
            context,
            &mut self.state,
            success,
            Confidence::Exact,
        );
    }
}
