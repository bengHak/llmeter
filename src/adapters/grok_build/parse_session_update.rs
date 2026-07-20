impl GrokBuildAdapter {
    fn parse_session_update(
        &mut self,
        output: &mut Vec<TelemetryEvent>,
        value: &Value,
        context: &AdapterContext,
    ) {
        let session = effective_session_id(value, context);
        let update_type = first_string_at(
            value,
            &[
                &["params", "update", "sessionUpdate"],
                &["params", "update", "type"],
            ],
        )
        .map(normalize_name)
        .unwrap_or_default();

        match update_type.as_str() {
            "usermessagechunk" => {
                if self.state.current_turn(&session).is_none() {
                    let turn = self.state.begin_turn(&session, turn_id(value));
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Derived,
                        EventKind::TurnStarted,
                    ));
                }
            }
            "agentmessagechunk" | "agentthoughtchunk" => {
                if let Some(text) = first_string_at(
                    value,
                    &[
                        &["params", "update", "content", "text"],
                        &["params", "update", "text"],
                        &["params", "update", "delta"],
                    ],
                ) {
                    push_output(
                        output,
                        &session,
                        text,
                        value,
                        context,
                        &mut self.state,
                        Confidence::Estimated,
                    );
                }
            }
            "toolcall" => {
                let turn = ensure_turn_started(
                    output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    Confidence::Derived,
                );
                let status = first_string_at(
                    value,
                    &[
                        &["params", "update", "status"],
                        &["params", "update", "state"],
                    ],
                )
                .map(normalize_name)
                .unwrap_or_else(|| "inprogress".to_owned());
                if is_finished_status(&status) {
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::ToolFinished {
                            call_id: grok_tool_call_id(value),
                            success: Some(status == "completed" || status == "success"),
                        },
                    ));
                } else {
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::ToolStarted {
                            call_id: grok_tool_call_id(value),
                            name: grok_tool_name(value),
                        },
                    ));
                }
            }
            "toolcallupdate" => {
                let status = first_string_at(
                    value,
                    &[
                        &["params", "update", "status"],
                        &["params", "update", "state"],
                    ],
                )
                .map(normalize_name)
                .unwrap_or_default();
                if is_finished_status(&status) {
                    let turn = ensure_turn_started(
                        output,
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
                        EventKind::ToolFinished {
                            call_id: grok_tool_call_id(value),
                            success: Some(status == "completed" || status == "success"),
                        },
                    ));
                }
            }
            "pendinginteraction" => {
                let turn = ensure_turn_started(
                    output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    Confidence::Derived,
                );
                let reason = first_string_at(
                    value,
                    &[
                        &["params", "update", "kind"],
                        &["params", "update", "reason"],
                    ],
                )
                .map(str::to_owned);
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::WaitingForInput { reason },
                ));
            }
            "retrystate" => {
                let state = first_string_at(
                    value,
                    &[
                        &["params", "update", "type"],
                        &["params", "update", "state"],
                    ],
                )
                .map(normalize_name)
                .unwrap_or_default();
                let turn = ensure_turn_started(
                    output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    Confidence::Derived,
                );
                if state == "retrying" || state == "waiting" {
                    if self.retrying_sessions.insert(session.clone()) {
                        output.push(event(
                            self.tool(),
                            &session,
                            Some(&turn),
                            value,
                            context,
                            Confidence::Exact,
                            EventKind::RetryStarted,
                        ));
                    }
                } else if self.retrying_sessions.remove(&session) {
                    let success = matches!(state.as_str(), "recovered" | "success" | "completed");
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::RetryFinished {
                            success: Some(success),
                        },
                    ));
                }
            }
            "modelchanged" => {
                let model = first_string_at(
                    value,
                    &[
                        &["params", "update", "model_id"],
                        &["params", "update", "modelId"],
                        &["params", "update", "model"],
                    ],
                );
                push_metadata(output, &session, value, context, model);
            }
            "turncompleted" => {
                let stop_reason = first_string_at(
                    value,
                    &[
                        &["params", "update", "stop_reason"],
                        &["params", "update", "stopReason"],
                    ],
                )
                .map(normalize_name)
                .unwrap_or_default();
                let success = !matches!(
                    stop_reason.as_str(),
                    "cancelled" | "canceled" | "error" | "failed" | "refused" | "maxturnsreached"
                );
                if self.retrying_sessions.remove(&session) {
                    let turn = ensure_turn_started(
                        output,
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
                        EventKind::RetryFinished {
                            success: Some(success),
                        },
                    ));
                }
                push_usage(output, &session, value, context, &self.state);
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
            "turncancelled" | "turnfailed" => {
                finish_turn(
                    output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    false,
                    Confidence::Exact,
                );
            }
            "sessionended" => output.push(event(
                self.tool(),
                &session,
                None,
                value,
                context,
                Confidence::Exact,
                EventKind::SessionEnded,
            )),
            _ => {}
        }
    }
}
