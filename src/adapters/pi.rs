use serde_json::Value;

use crate::adapters::common::{
    event, event_type, generic_usage, metadata_event, safe_output_delta, session_id, string_at,
    tool_call_id, tool_name, turn_id, ParserState,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct PiAdapter {
    state: ParserState,
}

impl Adapter for PiAdapter {
    fn tool(&self) -> ToolId {
        ToolId::Pi
    }

    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let mut output = Vec::new();
        let session = session_id(value, &context.fallback_session_id);
        let kind = event_type(value).unwrap_or_default();

        match kind {
            "agent_start" => {
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::SessionStarted,
                ));
                push_metadata(&mut output, self.tool(), &session, value, context);
            }
            "turn_start" => {
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
            "message_update" => {
                let delta_kind =
                    string_at(value, &["assistantMessageEvent", "type"]).unwrap_or_default();
                if matches!(delta_kind, "text_delta" | "thinking_delta") {
                    if let Some(delta) = string_at(value, &["assistantMessageEvent", "delta"]) {
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
            }
            "message_end" => {
                let usage = generic_usage(value);
                if !usage.is_empty() {
                    let turn = self.state.ensure_turn(&session, turn_id(value));
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
            "tool_execution_start" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let call_id = tool_call_id(value).unwrap_or_else(|| "pi-tool".to_owned());
                let name = tool_name(value).unwrap_or_else(|| "tool".to_owned());
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolStarted { call_id, name },
                ));
            }
            "tool_execution_end" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let call_id = tool_call_id(value).unwrap_or_else(|| "pi-tool".to_owned());
                let success = value
                    .get("isError")
                    .and_then(Value::as_bool)
                    .map(|error| !error);
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished { call_id, success },
                ));
            }
            "auto_retry_start" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
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
            "auto_retry_end" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let success = value.get("success").and_then(Value::as_bool);
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::RetryFinished { success },
                ));
            }
            "turn_end" | "agent_end" | "agent_settled" => {
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
            "extension_error" => {
                let turn = self.state.current_turn(&session);
                output.push(event(
                    self.tool(),
                    &session,
                    turn.as_deref(),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::Error {
                        code: Some("extension_error".to_owned()),
                    },
                ));
            }
            "response" => {
                let command = string_at(value, &["command"]).unwrap_or_default();
                if command == "get_session_stats" {
                    let usage = crate::adapters::common::UsageFields {
                        input_tokens: crate::adapters::common::u64_at(
                            value,
                            &["data", "tokens", "input"],
                        ),
                        output_tokens: crate::adapters::common::u64_at(
                            value,
                            &["data", "tokens", "output"],
                        ),
                        cached_input_tokens: crate::adapters::common::u64_at(
                            value,
                            &["data", "tokens", "cacheRead"],
                        ),
                        reasoning_tokens: None,
                        context_window: crate::adapters::common::u64_at(
                            value,
                            &["data", "contextUsage", "contextWindow"],
                        ),
                    };
                    if !usage.is_empty() {
                        output.push(event(
                            self.tool(),
                            &session,
                            self.state.current_turn(&session).as_deref(),
                            value,
                            context,
                            Confidence::Exact,
                            usage.into_event(true),
                        ));
                    }
                } else if command == "get_state" {
                    push_metadata(&mut output, self.tool(), &session, value, context);
                }
            }
            _ => {}
        }

        output
    }
}

fn push_metadata(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    value: &Value,
    context: &AdapterContext,
) {
    let kind = metadata_event(value).or_else(|| {
        let model = string_at(value, &["data", "model", "id"])
            .or_else(|| string_at(value, &["data", "model", "modelId"]))
            .map(str::to_owned);
        model.map(|model| EventKind::Metadata {
            cwd: None,
            model: Some(model),
            provider: string_at(value, &["data", "model", "provider"]).map(str::to_owned),
            pid: None,
        })
    });
    if let Some(kind) = kind {
        output.push(event(
            tool,
            session,
            None,
            value,
            context,
            Confidence::Exact,
            kind,
        ));
    }
}
