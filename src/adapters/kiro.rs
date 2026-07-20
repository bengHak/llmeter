use serde_json::Value;

use crate::adapters::common::{
    event, event_type, first_bool_at, first_string_at, metadata_event, safe_output_delta,
    session_id, string_at, tool_call_id, tool_name, turn_id, ParserState,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct KiroAdapter {
    state: ParserState,
}

impl Adapter for KiroAdapter {
    fn tool(&self) -> ToolId {
        ToolId::Kiro
    }

    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let session = session_id(value, &context.fallback_session_id);
        let kind = event_type(value).unwrap_or_default();
        let normalized = kind.to_ascii_lowercase();

        match normalized.as_str() {
            "agentspawn" | "sessionstart" | "session_start" => {
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
            "userpromptsubmit" | "user_prompt_submit" | "session/prompt" => {
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
            "pretooluse" | "pre_tool_use" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolStarted {
                        call_id: tool_call_id(value).unwrap_or_else(|| "kiro-tool".to_owned()),
                        name: tool_name(value).unwrap_or_else(|| "tool".to_owned()),
                    },
                ));
            }
            "posttooluse" | "post_tool_use" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id: tool_call_id(value).unwrap_or_else(|| "kiro-tool".to_owned()),
                        success: first_bool_at(value, &[&["success"]]).or(Some(true)),
                    },
                ));
            }
            "stop" => finish_turn(
                &mut output,
                self.tool(),
                &session,
                value,
                context,
                &mut self.state,
                true,
            ),
            "session/notification" => parse_notification(
                &mut output,
                self.tool(),
                &session,
                value,
                context,
                &mut self.state,
            ),
            "sessionend" | "session_end" | "_session/terminate" => output.push(event(
                self.tool(),
                &session,
                None,
                value,
                context,
                Confidence::Exact,
                EventKind::SessionEnded,
            )),
            _ => {
                if value.get("result").is_some()
                    && first_string_at(
                        value,
                        &[&["result", "sessionId"], &["result", "session_id"]],
                    )
                    .is_some()
                {
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
            }
        }

        output
    }
}

fn parse_notification(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
) {
    let update_type = first_string_at(
        value,
        &[
            &["params", "update", "sessionUpdate"],
            &["params", "update", "type"],
        ],
    )
    .unwrap_or_default();
    match update_type {
        "AgentMessageChunk" | "agent_message_chunk" => {
            if let Some(text) = first_string_at(
                value,
                &[
                    &["params", "update", "content", "text"],
                    &["params", "update", "text"],
                    &["params", "update", "delta"],
                ],
            ) {
                let turn = state.ensure_turn(session, turn_id(value));
                state.mark_output(session);
                output.push(event(
                    tool,
                    session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Estimated,
                    safe_output_delta(text),
                ));
            }
        }
        "ToolCall" | "tool_call" => {
            let turn = state.ensure_turn(session, turn_id(value));
            let status = string_at(value, &["params", "update", "status"]).unwrap_or("in_progress");
            if matches!(status, "completed" | "failed" | "error") {
                output.push(event(
                    tool,
                    session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id: tool_call_id(value).unwrap_or_else(|| "kiro-tool".to_owned()),
                        success: Some(status == "completed"),
                    },
                ));
            } else {
                output.push(event(
                    tool,
                    session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolStarted {
                        call_id: tool_call_id(value).unwrap_or_else(|| "kiro-tool".to_owned()),
                        name: tool_name(value).unwrap_or_else(|| "tool".to_owned()),
                    },
                ));
            }
        }
        "ToolCallUpdate" | "tool_call_update" => {
            let turn = state.ensure_turn(session, turn_id(value));
            let status = string_at(value, &["params", "update", "status"]).unwrap_or_default();
            if matches!(status, "completed" | "failed" | "error") {
                output.push(event(
                    tool,
                    session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id: tool_call_id(value).unwrap_or_else(|| "kiro-tool".to_owned()),
                        success: Some(status == "completed"),
                    },
                ));
            }
        }
        "TurnEnd" | "turn_end" => finish_turn(output, tool, session, value, context, state, true),
        _ => {}
    }
}

fn finish_turn(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
    success: bool,
) {
    if let Some(turn) = state.current_turn(session) {
        output.push(event(
            tool,
            session,
            Some(&turn),
            value,
            context,
            Confidence::Exact,
            EventKind::TurnFinished { success },
        ));
        state.end_turn(session);
    }
}
