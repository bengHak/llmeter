impl GrokBuildAdapter {
    fn parse_rpc(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let method = string_at(value, &["method"]).unwrap_or_default();

        match method {
            "session/new" => {
                let session = effective_session_id(value, context);
                if let Some(id) = request_id(value) {
                    self.pending_new_sessions.insert(id, session);
                } else {
                    output.push(event(
                        self.tool(),
                        &session,
                        None,
                        value,
                        context,
                        Confidence::Derived,
                        EventKind::SessionStarted,
                    ));
                    push_metadata(&mut output, &session, value, context, None);
                }
            }
            "session/prompt" => {
                let session = effective_session_id(value, context);
                let turn = self.state.begin_turn(&session, turn_id(value));
                if let Some(id) = request_id(value) {
                    self.pending_prompts.insert(id, session.clone());
                }
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::TurnStarted,
                ));
            }
            "session/update" | "_x.ai/session/update" => {
                self.parse_session_update(&mut output, value, context);
            }
            "session/terminate" | "_session/terminate" => {
                let session = effective_session_id(value, context);
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::SessionEnded,
                ));
            }
            "" => self.parse_rpc_response(&mut output, value, context),
            _ => {}
        }

        output
    }
}
