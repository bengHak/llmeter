use serde_json::Value;

use crate::adapters::common::{
    collect_texts_at, event, event_type, first_bool_at, first_string_at, metadata_event,
    safe_output_delta, session_id, string_at, tool_call_id, tool_name, turn_id, u64_at,
    ParserState, UsageFields,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct CodexAdapter {
    state: ParserState,
}

impl Adapter for CodexAdapter {
    fn tool(&self) -> ToolId {
        ToolId::Codex
    }

    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let mut session = session_id(value, &context.fallback_session_id);
        let top_kind = event_type(value).unwrap_or_default();

        if top_kind == "session_meta" {
            if let Some(native_id) =
                first_string_at(value, &[&["payload", "id"], &["payload", "session_id"]])
            {
                session = native_id.to_owned();
            }
        }

        match top_kind {
            "thread.started" | "thread_started" | "session_meta" => {
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::SessionStarted,
                ));
                let metadata_source = value.get("payload").unwrap_or(value);
                if let Some(metadata) = metadata_event(metadata_source) {
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
            "turn.started" | "turn_started" => {
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
            "item.started" | "item_started" => {
                if is_tool_item(value) {
                    let turn = self.state.ensure_turn(&session, turn_id(value));
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::ToolStarted {
                            call_id: item_id(value).unwrap_or_else(|| "codex-tool".to_owned()),
                            name: item_tool_name(value),
                        },
                    ));
                }
            }
            "item.completed" | "item_completed" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let item_type = first_string_at(value, &[&["item", "type"], &["item_type"]])
                    .unwrap_or_default();
                if item_type == "agent_message" || item_type == "assistant_message" {
                    if let Some(text) = first_string_at(
                        value,
                        &[&["item", "text"], &["item", "content"], &["text"]],
                    ) {
                        if !text.is_empty() {
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
                } else if is_tool_item(value) {
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        value,
                        context,
                        Confidence::Exact,
                        EventKind::ToolFinished {
                            call_id: item_id(value).unwrap_or_else(|| "codex-tool".to_owned()),
                            success: first_bool_at(value, &[&["item", "success"], &["success"]])
                                .or(Some(true)),
                        },
                    ));
                }
            }
            "turn.completed" | "turn_completed" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let usage = usage_from_exec(value);
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
            "turn.failed" | "turn_failed" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::Error {
                        code: first_string_at(value, &[&["error", "code"], &["code"]])
                            .map(str::to_owned),
                    },
                ));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::TurnFinished { success: false },
                ));
                self.state.end_turn(&session);
            }
            "turn_context" => {
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
                if let Some(metadata) = value
                    .get("payload")
                    .and_then(metadata_event)
                    .or_else(|| metadata_event(value))
                {
                    output.push(event(
                        self.tool(),
                        &session,
                        self.state.current_turn(&session).as_deref(),
                        value,
                        context,
                        Confidence::Exact,
                        metadata,
                    ));
                }
            }
            "event_msg" => parse_event_message(
                &mut output,
                self.tool(),
                &session,
                value,
                context,
                &mut self.state,
            ),
            "response_item" => parse_response_item(
                &mut output,
                self.tool(),
                &session,
                value,
                context,
                &mut self.state,
            ),
            _ => {}
        }

        output
    }
}

fn usage_from_exec(value: &Value) -> UsageFields {
    UsageFields {
        input_tokens: u64_at(value, &["usage", "input_tokens"]),
        output_tokens: u64_at(value, &["usage", "output_tokens"]),
        cached_input_tokens: u64_at(value, &["usage", "cached_input_tokens"]),
        reasoning_tokens: u64_at(value, &["usage", "reasoning_output_tokens"]),
        context_window: u64_at(value, &["usage", "context_window"]),
    }
}

fn parse_event_message(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
) {
    let payload_kind = string_at(value, &["payload", "type"]).unwrap_or_default();
    match payload_kind {
        "token_count" => {
            let usage = UsageFields {
                input_tokens: u64_at(
                    value,
                    &["payload", "info", "total_token_usage", "input_tokens"],
                ),
                output_tokens: u64_at(
                    value,
                    &["payload", "info", "total_token_usage", "output_tokens"],
                ),
                cached_input_tokens: u64_at(
                    value,
                    &[
                        "payload",
                        "info",
                        "total_token_usage",
                        "cached_input_tokens",
                    ],
                ),
                reasoning_tokens: u64_at(
                    value,
                    &[
                        "payload",
                        "info",
                        "total_token_usage",
                        "reasoning_output_tokens",
                    ],
                ),
                context_window: u64_at(value, &["payload", "info", "model_context_window"]),
            };
            if !usage.is_empty() {
                output.push(event(
                    tool,
                    session,
                    state.current_turn(session).as_deref(),
                    value,
                    context,
                    Confidence::Exact,
                    usage.into_event(true),
                ));
            }
        }
        "task_started" | "turn_started" => {
            let turn = state.begin_turn(session, turn_id(value));
            output.push(event(
                tool,
                session,
                Some(&turn),
                value,
                context,
                Confidence::Exact,
                EventKind::TurnStarted,
            ));
        }
        "task_complete" | "turn_complete" => {
            if let Some(turn) = state.current_turn(session) {
                output.push(event(
                    tool,
                    session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::TurnFinished { success: true },
                ));
                state.end_turn(session);
            }
        }
        "agent_message" => {
            if let Some(text) = string_at(value, &["payload", "message"]) {
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
        "error" => output.push(event(
            tool,
            session,
            state.current_turn(session).as_deref(),
            value,
            context,
            Confidence::Exact,
            EventKind::Error {
                code: string_at(value, &["payload", "code"]).map(str::to_owned),
            },
        )),
        _ => {}
    }
}

fn parse_response_item(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
) {
    let payload = value.get("payload").unwrap_or(value);
    let payload_type = string_at(payload, &["type"]).unwrap_or_default();
    let turn = state.ensure_turn(session, turn_id(value));

    match payload_type {
        "message" if string_at(payload, &["role"]) == Some("assistant") => {
            if !state.saw_output(session) {
                let mut texts = Vec::new();
                collect_texts_at(payload, &["content"], &mut texts);
                for text in texts.into_iter().filter(|text| !text.is_empty()) {
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
                state.mark_output(session);
            }
        }
        "function_call" | "custom_tool_call" | "local_shell_call" => {
            output.push(event(
                tool,
                session,
                Some(&turn),
                value,
                context,
                Confidence::Exact,
                EventKind::ToolStarted {
                    call_id: tool_call_id(payload).unwrap_or_else(|| "codex-tool".to_owned()),
                    name: tool_name(payload).unwrap_or_else(|| payload_type.to_owned()),
                },
            ));
        }
        "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" => {
            output.push(event(
                tool,
                session,
                Some(&turn),
                value,
                context,
                Confidence::Exact,
                EventKind::ToolFinished {
                    call_id: tool_call_id(payload).unwrap_or_else(|| "codex-tool".to_owned()),
                    success: Some(true),
                },
            ));
        }
        _ => {}
    }
}

fn is_tool_item(value: &Value) -> bool {
    matches!(
        first_string_at(value, &[&["item", "type"], &["type"]]),
        Some(
            "command_execution" | "file_change" | "mcp_tool_call" | "web_search" | "function_call"
        )
    )
}

fn item_id(value: &Value) -> Option<String> {
    first_string_at(
        value,
        &[&["item", "id"], &["item", "call_id"], &["call_id"]],
    )
    .map(str::to_owned)
}

fn item_tool_name(value: &Value) -> String {
    first_string_at(
        value,
        &[&["item", "name"], &["item", "command"], &["item", "type"]],
    )
    .unwrap_or("tool")
    .to_owned()
}
