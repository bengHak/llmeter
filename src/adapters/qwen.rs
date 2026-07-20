use serde_json::Value;

use crate::adapters::common::{
    collect_texts_at, event, event_type, first_bool_at, first_string_at, generic_usage,
    metadata_event, safe_output_delta, session_id, string_at, turn_id, ParserState,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct QwenAdapter {
    state: ParserState,
}

impl Adapter for QwenAdapter {
    fn tool(&self) -> ToolId {
        ToolId::Qwen
    }

    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let session = session_id(value, &context.fallback_session_id);
        let kind = event_type(value).unwrap_or_default();
        let subtype = string_at(value, &["subtype"]).unwrap_or_default();

        match kind {
            "system" if subtype == "session_start" => {
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
            "message_start" => {
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
            "content_block_delta" | "message_delta" => {
                if let Some(text) = first_string_at(
                    value,
                    &[
                        &["delta", "text"],
                        &["event", "delta", "text"],
                        &["event", "delta"],
                    ],
                ) {
                    if !text.is_empty() {
                        let turn = self.state.ensure_turn(&session, turn_id(value));
                        self.state.mark_output(&session);
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
                }
            }
            "assistant" => {
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
                parse_assistant_tools(&mut output, self.tool(), &session, &turn, value, context);
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
                if let Some(metadata) = metadata_event(value) {
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        metadata,
                    ));
                }
            }
            "tool_result" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let call_id = first_string_at(value, &[&["tool_use_id"], &["toolUseId"], &["id"]])
                    .unwrap_or("qwen-tool")
                    .to_owned();
                let success =
                    !first_bool_at(value, &[&["is_error"], &["isError"]]).unwrap_or(false);
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id,
                        success: Some(success),
                    },
                ));
            }
            "result" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
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
                let success = subtype != "error"
                    && !first_bool_at(value, &[&["is_error"], &["isError"]]).unwrap_or(false);
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
            "session_end" => output.push(event(
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

        output
    }
}

fn parse_assistant_tools(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    turn: &str,
    value: &Value,
    context: &AdapterContext,
) {
    let Some(content) = value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for item in content {
        if string_at(item, &["type"]) != Some("tool_use") {
            continue;
        }
        output.push(event(
            tool,
            session,
            Some(turn),
            value,
            context,
            Confidence::Exact,
            EventKind::ToolStarted {
                call_id: first_string_at(item, &[&["id"], &["tool_use_id"]])
                    .unwrap_or("qwen-tool")
                    .to_owned(),
                name: string_at(item, &["name"]).unwrap_or("tool").to_owned(),
            },
        ));
    }
}
