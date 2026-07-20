impl GrokBuildAdapter {
    fn parse_hook(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let session = effective_session_id(value, context);
        let normalized = normalize_name(hook_name(value).unwrap_or_default());

        match normalized.as_str() {
            "sessionstart" => {
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::SessionStarted,
                ));
                push_metadata(&mut output, &session, value, context, None);
            }
            "userpromptsubmit" | "beforesubmitprompt" => {
                let turn = self.state.begin_turn(&session, turn_id(value));
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
            "pretooluse" | "beforeshellexecution" | "beforemcpexecution" | "beforereadfile" => {
                let turn = ensure_turn_started(
                    &mut output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    Confidence::Exact,
                );
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
            "posttooluse"
            | "aftershellexecution"
            | "aftermcpexecution"
            | "afterfileedit" => {
                finish_tool(&mut output, &session, value, context, &mut self.state, true);
            }
            "posttoolusefailure" => {
                finish_tool(&mut output, &session, value, context, &mut self.state, false);
            }
            "permissiondenied" => {
                finish_tool(&mut output, &session, value, context, &mut self.state, false);
                let turn = self.state.current_turn(&session).or_else(|| turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    turn.as_deref(),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::Error {
                        code: Some("permission_denied".to_owned()),
                    },
                ));
            }
            "notification" | "permissionrequest" => {
                let reason = first_string_at(
                    value,
                    &[
                        &["notificationType"],
                        &["notification_type"],
                        &["level"],
                        &["reason"],
                    ],
                )
                .unwrap_or("notification");
                let normalized_reason = normalize_name(reason);
                if normalized == "permissionrequest"
                    || normalized_reason.contains("permission")
                    || normalized_reason.contains("input")
                    || normalized_reason.contains("idle")
                {
                    let turn = ensure_turn_started(
                        &mut output,
                        &session,
                        value,
                        context,
                        &mut self.state,
                        Confidence::Exact,
                    );
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::WaitingForInput {
                            reason: Some(reason.to_owned()),
                        },
                    ));
                }
            }
            "stop" | "subagentstop" | "subagentend" => {
                finish_turn(
                    &mut output,
                    &session,
                    value,
                    context,
                    &mut self.state,
                    true,
                    Confidence::Exact,
                );
            }
            "stopfailure" => {
                let turn = self.state.current_turn(&session).or_else(|| turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    turn.as_deref(),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::Error {
                        code: Some("grok_stop_failure".to_owned()),
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
            "subagentstart" => {
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::SessionStarted,
                ));
            }
            "sessionend" => {
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
            _ => {}
        }

        output
    }
}
