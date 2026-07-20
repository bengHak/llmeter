use serde_json::Value;

use crate::adapters::common::{
    collect_texts_at, event, event_type, generic_usage, metadata_event, safe_output_delta,
    session_id, string_at, tool_call_id, tool_name, turn_id, ParserState,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct ClaudeAdapter {
    state: ParserState,
}

impl Adapter for ClaudeAdapter {
    fn tool(&self) -> ToolId {
        ToolId::Claude
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
            "userpromptsubmit" | "user_prompt_submit" => {
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
                        call_id: tool_call_id(value).unwrap_or_else(|| "claude-tool".to_owned()),
                        name: tool_name(value).unwrap_or_else(|| "tool".to_owned()),
                    },
                ));
            }
            "posttooluse" | "post_tool_use" | "posttoolusefailure" | "post_tool_use_failure" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id: tool_call_id(value).unwrap_or_else(|| "claude-tool".to_owned()),
                        success: Some(!normalized.contains("failure")),
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
            "stop" | "subagentstop" | "subagent_stop" => {
                if let Some(turn) = self.state.current_turn(&session).or_else(|| turn_id(value)) {
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::TurnFinished { success: true },
                    ));
                    self.state.end_turn(&session);
                }
            }
            "sessionend" | "session_end" => output.push(event(
                self.tool(),
                &session,
                None,
                value,
                context,
                Confidence::Exact,
                EventKind::SessionEnded,
            )),
            "assistant" if value.get("hook_event_name").is_none() => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                if !self.state.saw_output(&session) {
                    let mut texts = Vec::new();
                    collect_texts_at(value, &["message", "content"], &mut texts);
                    for text in texts.into_iter().filter(|text| !text.is_empty()) {
                        output.push(event(
                            self.tool(),
                            &session,
                            Some(&turn),
                            value,
                            context,
                            Confidence::Estimated,
                            safe_output_delta(text),
                        ));
                    }
                    self.state.mark_output(&session);
                }
                let usage = generic_usage(value);
                if !usage.is_empty() {
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        usage.into_event(false),
                    ));
                }
            }
            "user" if value.get("hook_event_name").is_none() => {
                let is_tool_result =
                    string_at(value, &["message", "content", "0", "type"]) == Some("tool_result");
                if !is_tool_result {
                    let turn = self.state.begin_turn(&session, turn_id(value));
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Estimated,
                        EventKind::TurnStarted,
                    ));
                }
            }
            _ => {}
        }

        output
    }
}
