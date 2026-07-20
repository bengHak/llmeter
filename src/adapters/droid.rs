use serde_json::Value;

use crate::adapters::common::{
    event, event_type, first_bool_at, first_string_at, generic_usage, metadata_event,
    safe_output_delta, session_id, tool_call_id, tool_name, turn_id, ParserState,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct DroidAdapter {
    state: ParserState,
}

impl Adapter for DroidAdapter {
    fn tool(&self) -> ToolId {
        ToolId::Droid
    }

    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let session = session_id(value, &context.fallback_session_id);
        let kind = event_type(value).unwrap_or_default();
        let normalized = kind.to_ascii_lowercase();

        match normalized.as_str() {
            "sessionstart" | "session_start" => {
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::SessionStarted,
                ));
                if let Some(metadata) = metadata_event(value) {
                    output.push(event(
                        self.tool(),
                        &session,
                        None,
                        value,
                        context,
                        Confidence::Exact,
                        metadata,
                    ));
                }
            }
            "userpromptsubmit" | "user_prompt_submit" | "turn.started" | "turn_start" => {
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
            "pretooluse" | "pre_tool_use" | "tool.started" | "tool_start" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolStarted {
                        call_id: tool_call_id(value).unwrap_or_else(|| "droid-tool".to_owned()),
                        name: tool_name(value).unwrap_or_else(|| "tool".to_owned()),
                    },
                ));
            }
            "posttooluse" | "post_tool_use" | "posttoolusefailure" | "tool.finished"
            | "tool_end" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let success = if normalized.contains("failure") {
                    Some(false)
                } else {
                    first_bool_at(value, &[&["success"], &["is_success"]]).or(Some(true))
                };
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id: tool_call_id(value).unwrap_or_else(|| "droid-tool".to_owned()),
                        success,
                    },
                ));
            }
            "permissionrequest" | "permission_request" | "notification" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::WaitingForInput {
                        reason: Some("permission".to_owned()),
                    },
                ));
            }
            "stop" | "subagentstop" | "turn.completed" | "turn_end" | "result" => {
                push_usage(
                    &mut output,
                    self.tool(),
                    &session,
                    value,
                    context,
                    &mut self.state,
                );
                if let Some(turn) = self.state.current_turn(&session).or_else(|| turn_id(value)) {
                    let success = !first_bool_at(value, &[&["is_error"]]).unwrap_or(false);
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::TurnFinished { success },
                    ));
                    self.state.end_turn(&session);
                }
            }
            "sessionend" | "session_end" => {
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
            _ => {
                if normalized.contains("delta") || normalized.contains("chunk") {
                    if let Some(delta) = first_string_at(
                        value,
                        &[
                            &["delta"],
                            &["text"],
                            &["content", "text"],
                            &["params", "delta"],
                            &["params", "text"],
                            &["event", "delta"],
                        ],
                    ) {
                        let turn = self.state.ensure_turn(&session, turn_id(value));
                        self.state.mark_output(&session);
                        output.push(event(
                            self.tool(),
                            &session,
                            Some(&turn),
                            value,
                            context,
                            Confidence::Estimated,
                            safe_output_delta(delta),
                        ));
                    }
                }
                push_usage(
                    &mut output,
                    self.tool(),
                    &session,
                    value,
                    context,
                    &mut self.state,
                );
            }
        }

        output
    }
}

fn push_usage(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
) {
    let usage = generic_usage(value);
    if usage.is_empty() {
        return;
    }
    let turn = state.ensure_turn(session, turn_id(value));
    output.push(event(
        tool,
        session,
        Some(&turn),
        value,
        context,
        Confidence::Exact,
        usage.into_event(false),
    ));
}
