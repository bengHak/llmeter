use serde_json::Value;

use crate::adapters::common::{
    collect_texts_at, event, event_type, first_string_at, generic_usage, metadata_event,
    safe_output_delta, session_id, string_at, tool_call_id, tool_name, turn_id, ParserState,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct GeminiAdapter {
    state: ParserState,
}

impl Adapter for GeminiAdapter {
    fn tool(&self) -> ToolId {
        ToolId::Gemini
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
            "beforeagent" | "before_agent" => {
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
            "aftermodel" | "after_model" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                let mut texts = Vec::new();
                collect_texts_at(value, &["llm_response", "candidates"], &mut texts);
                if texts.is_empty() {
                    if let Some(text) = first_string_at(
                        value,
                        &[
                            &["llm_response", "text"],
                            &["llm_response", "delta"],
                            &["response", "text"],
                        ],
                    ) {
                        texts.push(text);
                    }
                }
                for text in texts.into_iter().filter(|text| !text.is_empty()) {
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
                push_usage(&mut output, self.tool(), &session, &turn, value, context);
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
            "beforetool" | "before_tool" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolStarted {
                        call_id: tool_call_id(value).unwrap_or_else(|| "gemini-tool".to_owned()),
                        name: tool_name(value).unwrap_or_else(|| "tool".to_owned()),
                    },
                ));
            }
            "aftertool" | "after_tool" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id: tool_call_id(value).unwrap_or_else(|| "gemini-tool".to_owned()),
                        success: Some(true),
                    },
                ));
            }
            "afteragent" | "after_agent" => {
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
            "notification" => {
                let turn = self.state.ensure_turn(&session, turn_id(value));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    value,
                    context,
                    Confidence::Exact,
                    EventKind::WaitingForInput {
                        reason: Some("notification".to_owned()),
                    },
                ));
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
            _ => parse_telemetry_record(
                &mut output,
                self.tool(),
                &session,
                value,
                context,
                &mut self.state,
            ),
        }

        output
    }
}

fn push_usage(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    turn: &str,
    value: &Value,
    context: &AdapterContext,
) {
    let usage = generic_usage(value);
    if !usage.is_empty() {
        output.push(event(
            tool,
            session,
            Some(turn),
            value,
            context,
            Confidence::Exact,
            usage.into_event(false),
        ));
    }
}

fn parse_telemetry_record(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    value: &Value,
    context: &AdapterContext,
    state: &mut ParserState,
) {
    let metric_name = first_string_at(
        value,
        &[
            &["name"],
            &["metric"],
            &["metric_name"],
            &["instrumentationScope", "name"],
        ],
    )
    .unwrap_or_default();
    let operation = first_string_at(
        value,
        &[
            &["attributes", "gen_ai.operation.name"],
            &["span", "attributes", "gen_ai.operation.name"],
        ],
    );

    if operation == Some("agent_call") && string_at(value, &["phase"]) == Some("start") {
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

    let mut usage = generic_usage(value);
    if metric_name.contains("token.usage") || metric_name.contains("token_usage") {
        let token_type = first_string_at(
            value,
            &[
                &["attributes", "type"],
                &["labels", "type"],
                &["dataPoint", "attributes", "type"],
            ],
        );
        let amount = crate::adapters::common::first_u64_at(
            value,
            &[
                &["value"],
                &["sum"],
                &["asInt"],
                &["dataPoint", "value"],
                &["dataPoint", "asInt"],
            ],
        );
        match token_type {
            Some("input") => usage.input_tokens = amount,
            Some("output") => usage.output_tokens = amount,
            Some("cache") => usage.cached_input_tokens = amount,
            Some("thought") => usage.reasoning_tokens = amount,
            _ => {}
        }
    }
    if !usage.is_empty() {
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

    if let Some(metadata) = metadata_event(value) {
        output.push(event(
            tool,
            session,
            state.current_turn(session).as_deref(),
            value,
            context,
            Confidence::Exact,
            metadata,
        ));
    }
}
