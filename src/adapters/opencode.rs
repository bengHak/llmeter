use serde_json::Value;

use crate::adapters::common::{
    event, event_type, first_bool_at, first_string_at, metadata_event, safe_output_delta,
    session_id, string_at, tool_call_id, tool_name, turn_id, ParserState,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct OpenCodeAdapter {
    state: ParserState,
}

impl Adapter for OpenCodeAdapter {
    fn tool(&self) -> ToolId {
        ToolId::OpenCode
    }

    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        let body = value
            .get("payload")
            .filter(|payload| payload.get("type").is_some())
            .unwrap_or(value);
        let kind = event_type(body).unwrap_or_default();
        let mut session = session_id(body, &context.fallback_session_id);
        if kind == "session.created" {
            if let Some(id) = first_string_at(
                body,
                &[&["properties", "info", "id"], &["properties", "sessionID"]],
            ) {
                session = id.to_owned();
            }
        }
        let mut output = Vec::new();

        match kind {
            "session.created" => {
                output.push(event(
                    self.tool(),
                    &session,
                    None,
                    body,
                    context,
                    Confidence::Exact,
                    EventKind::SessionStarted,
                ));
                if let Some(metadata) = metadata_event(body) {
                    output.push(event(
                        self.tool(),
                        &session,
                        None,
                        body,
                        context,
                        Confidence::Exact,
                        metadata,
                    ));
                }
            }
            "session.updated" => {
                if let Some(metadata) = metadata_event(body) {
                    output.push(event(
                        self.tool(),
                        &session,
                        self.state.current_turn(&session).as_deref(),
                        body,
                        context,
                        Confidence::Exact,
                        metadata,
                    ));
                }
            }
            "message.updated" => {
                let role = string_at(body, &["properties", "info", "role"]).unwrap_or_default();
                if role == "user" || self.state.current_turn(&session).is_none() {
                    let supplied_turn = first_string_at(
                        body,
                        &[&["properties", "info", "id"], &["properties", "messageID"]],
                    )
                    .map(str::to_owned);
                    let turn = self.state.begin_turn(&session, supplied_turn);
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        body,
                        context,
                        Confidence::Exact,
                        EventKind::TurnStarted,
                    ));
                }
                if let Some(metadata) = metadata_event(body) {
                    output.push(event(
                        self.tool(),
                        &session,
                        self.state.current_turn(&session).as_deref(),
                        body,
                        context,
                        Confidence::Exact,
                        metadata,
                    ));
                }
            }
            "message.part.updated" => {
                let turn = self.state.ensure_turn(&session, turn_id(body));
                if let Some(delta) = string_at(body, &["properties", "delta"]) {
                    if !delta.is_empty() {
                        self.state.mark_output(&session);
                        output.push(event(
                            self.tool(),
                            &session,
                            Some(&turn),
                            body,
                            context,
                            Confidence::Estimated,
                            safe_output_delta(delta),
                        ));
                    }
                }
                parse_part_tool(&mut output, self.tool(), &session, &turn, body, context);
            }
            "tool.execute.before" => {
                let turn = self.state.ensure_turn(&session, turn_id(body));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    body,
                    context,
                    Confidence::Exact,
                    EventKind::ToolStarted {
                        call_id: tool_call_id(body).unwrap_or_else(|| "opencode-tool".to_owned()),
                        name: tool_name(body).unwrap_or_else(|| "tool".to_owned()),
                    },
                ));
            }
            "tool.execute.after" => {
                let turn = self.state.ensure_turn(&session, turn_id(body));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    body,
                    context,
                    Confidence::Exact,
                    EventKind::ToolFinished {
                        call_id: tool_call_id(body).unwrap_or_else(|| "opencode-tool".to_owned()),
                        success: first_bool_at(body, &[&["properties", "success"], &["success"]])
                            .or(Some(true)),
                    },
                ));
            }
            "permission.asked" | "question.asked" => {
                let turn = self.state.ensure_turn(&session, turn_id(body));
                output.push(event(
                    self.tool(),
                    &session,
                    Some(&turn),
                    body,
                    context,
                    Confidence::Exact,
                    EventKind::WaitingForInput {
                        reason: Some("permission".to_owned()),
                    },
                ));
            }
            "session.idle" => finish_turn(
                &mut output,
                self.tool(),
                &session,
                body,
                context,
                &mut self.state,
                true,
            ),
            "session.error" => {
                output.push(event(
                    self.tool(),
                    &session,
                    self.state.current_turn(&session).as_deref(),
                    body,
                    context,
                    Confidence::Exact,
                    EventKind::Error {
                        code: first_string_at(
                            body,
                            &[
                                &["properties", "error", "name"],
                                &["properties", "error", "code"],
                            ],
                        )
                        .map(str::to_owned),
                    },
                ));
                finish_turn(
                    &mut output,
                    self.tool(),
                    &session,
                    body,
                    context,
                    &mut self.state,
                    false,
                );
            }
            "session.deleted" => output.push(event(
                self.tool(),
                &session,
                None,
                body,
                context,
                Confidence::Exact,
                EventKind::SessionEnded,
            )),
            "session.status" => {
                let status = first_string_at(
                    body,
                    &[&["properties", "status", "type"], &["properties", "status"]],
                )
                .unwrap_or_default();
                if status == "retry" {
                    let turn = self.state.ensure_turn(&session, turn_id(body));
                    output.push(event(
                        self.tool(),
                        &session,
                        Some(&turn),
                        body,
                        context,
                        Confidence::Exact,
                        EventKind::RetryStarted,
                    ));
                }
            }
            _ => {}
        }

        output
    }
}

fn parse_part_tool(
    output: &mut Vec<TelemetryEvent>,
    tool: ToolId,
    session: &str,
    turn: &str,
    value: &Value,
    context: &AdapterContext,
) {
    if string_at(value, &["properties", "part", "type"]) != Some("tool") {
        return;
    }
    let status = first_string_at(
        value,
        &[
            &["properties", "part", "state", "status"],
            &["properties", "part", "status"],
        ],
    )
    .unwrap_or_default();
    let call_id = first_string_at(
        value,
        &[
            &["properties", "part", "callID"],
            &["properties", "part", "id"],
        ],
    )
    .unwrap_or("opencode-tool")
    .to_owned();
    match status {
        "pending" | "running" => output.push(event(
            tool,
            session,
            Some(turn),
            value,
            context,
            Confidence::Exact,
            EventKind::ToolStarted {
                call_id,
                name: string_at(value, &["properties", "part", "tool"])
                    .unwrap_or("tool")
                    .to_owned(),
            },
        )),
        "completed" | "error" => output.push(event(
            tool,
            session,
            Some(turn),
            value,
            context,
            Confidence::Exact,
            EventKind::ToolFinished {
                call_id,
                success: Some(status == "completed"),
            },
        )),
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
