use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::adapters::AdapterContext;
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Clone, Debug, Default)]
struct SessionParseState {
    current_turn: Option<String>,
    turn_counter: u64,
    saw_output: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ParserState {
    sessions: HashMap<String, SessionParseState>,
}

impl ParserState {
    pub fn begin_turn(&mut self, session_id: &str, supplied: Option<String>) -> String {
        let session = self.sessions.entry(session_id.to_owned()).or_default();
        session.turn_counter = session.turn_counter.saturating_add(1);
        let turn_id = supplied.unwrap_or_else(|| {
            format!(
                "{}-turn-{}",
                sanitize_identifier(session_id),
                session.turn_counter
            )
        });
        session.current_turn = Some(turn_id.clone());
        session.saw_output = false;
        turn_id
    }

    pub fn ensure_turn(&mut self, session_id: &str, supplied: Option<String>) -> String {
        if let Some(supplied) = supplied {
            let session = self.sessions.entry(session_id.to_owned()).or_default();
            if session.current_turn.as_deref() != Some(supplied.as_str()) {
                session.current_turn = Some(supplied.clone());
                session.saw_output = false;
            }
            return supplied;
        }
        if let Some(turn_id) = self
            .sessions
            .get(session_id)
            .and_then(|session| session.current_turn.clone())
        {
            return turn_id;
        }
        self.begin_turn(session_id, None)
    }

    pub fn current_turn(&self, session_id: &str) -> Option<String> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.current_turn.clone())
    }

    pub fn end_turn(&mut self, session_id: &str) -> Option<String> {
        self.sessions
            .get_mut(session_id)
            .and_then(|session| session.current_turn.take())
    }

    pub fn mark_output(&mut self, session_id: &str) {
        self.sessions
            .entry(session_id.to_owned())
            .or_default()
            .saw_output = true;
    }

    pub fn saw_output(&self, session_id: &str) -> bool {
        self.sessions
            .get(session_id)
            .is_some_and(|session| session.saw_output)
    }
}

pub fn sanitize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

pub fn event(
    tool: ToolId,
    session_id: &str,
    turn_id: Option<&str>,
    source: &Value,
    context: &AdapterContext,
    confidence: Confidence,
    kind: EventKind,
) -> TelemetryEvent {
    let occurred_at = source_timestamp(source).unwrap_or(context.observed_at);
    let mut event = TelemetryEvent::new(tool, session_id, occurred_at, confidence, kind)
        .with_observed_at(context.observed_at);
    if let Some(turn_id) = turn_id {
        event = event.with_turn_id(turn_id);
    }
    event
}

pub fn source_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    for path in [
        &["timestamp"][..],
        &["ts"][..],
        &["time"][..],
        &["created_at"][..],
        &["createdAt"][..],
        &["properties", "time", "created"][..],
    ] {
        let Some(value) = value_at(value, path) else {
            continue;
        };
        if let Some(text) = value.as_str() {
            if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
                return Some(parsed.with_timezone(&Utc));
            }
        }
        if let Some(number) = value.as_i64() {
            let millis = if number.abs() < 10_000_000_000 {
                number.saturating_mul(1_000)
            } else {
                number
            };
            if let Some(parsed) = Utc.timestamp_millis_opt(millis).single() {
                return Some(parsed);
            }
        }
    }
    None
}

pub fn event_type(value: &Value) -> Option<&str> {
    string_at(value, &["hook_event_name"])
        .or_else(|| string_at(value, &["type"]))
        .or_else(|| string_at(value, &["event", "type"]))
        .or_else(|| string_at(value, &["method"]))
}

pub fn session_id(value: &Value, fallback: &str) -> String {
    for path in [
        &["session_id"][..],
        &["sessionId"][..],
        &["sessionID"][..],
        &["thread_id"][..],
        &["threadId"][..],
        &["conversation_id"][..],
        &["conversationId"][..],
        &["params", "sessionId"][..],
        &["params", "session_id"][..],
        &["params", "update", "sessionId"][..],
        &["properties", "sessionID"][..],
        &["properties", "sessionId"][..],
        &["properties", "part", "sessionID"][..],
        &["properties", "info", "sessionID"][..],
        &["payload", "session_id"][..],
        &["message", "session_id"][..],
    ] {
        if let Some(id) = string_at(value, path) {
            if !id.is_empty() {
                return id.to_owned();
            }
        }
    }
    fallback.to_owned()
}

pub fn turn_id(value: &Value) -> Option<String> {
    for path in [
        &["turn_id"][..],
        &["turnId"][..],
        &["prompt_id"][..],
        &["promptId"][..],
        &["params", "turnId"][..],
        &["properties", "turnID"][..],
        &["message", "id"][..],
    ] {
        if let Some(id) = string_at(value, path) {
            if !id.is_empty() {
                return Some(id.to_owned());
            }
        }
    }
    None
}

pub fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    value_at(value, path)?.as_str()
}

pub fn bool_at(value: &Value, path: &[&str]) -> Option<bool> {
    value_at(value, path)?.as_bool()
}

pub fn u64_at(value: &Value, path: &[&str]) -> Option<u64> {
    let value = value_at(value, path)?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
        .or_else(|| {
            value
                .as_f64()
                .filter(|number| *number >= 0.0)
                .map(|number| number as u64)
        })
}

pub fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| match current {
        Value::Object(object) => object.get(*key),
        Value::Array(values) => key
            .parse::<usize>()
            .ok()
            .and_then(|index| values.get(index)),
        _ => None,
    })
}

pub fn first_string_at<'a>(value: &'a Value, paths: &[&[&str]]) -> Option<&'a str> {
    paths.iter().find_map(|path| string_at(value, path))
}

pub fn first_u64_at(value: &Value, paths: &[&[&str]]) -> Option<u64> {
    paths.iter().find_map(|path| u64_at(value, path))
}

pub fn first_bool_at(value: &Value, paths: &[&[&str]]) -> Option<bool> {
    paths.iter().find_map(|path| bool_at(value, path))
}

pub fn text_stats(text: &str) -> (u64, u64) {
    (text.chars().count() as u64, text.len() as u64)
}

pub fn safe_output_delta(text: &str) -> EventKind {
    let (characters, bytes) = text_stats(text);
    EventKind::OutputDelta {
        tokens: None,
        characters,
        bytes,
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UsageFields {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub context_window: Option<u64>,
}

impl UsageFields {
    pub fn is_empty(self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cached_input_tokens.is_none()
            && self.reasoning_tokens.is_none()
            && self.context_window.is_none()
    }

    pub fn into_event(self, cumulative: bool) -> EventKind {
        EventKind::Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_input_tokens: self.cached_input_tokens,
            reasoning_tokens: self.reasoning_tokens,
            context_window: self.context_window,
            cumulative,
        }
    }
}

pub fn generic_usage(value: &Value) -> UsageFields {
    UsageFields {
        input_tokens: first_u64_at(
            value,
            &[
                &["usage", "input_tokens"],
                &["usage", "inputTokens"],
                &["usage", "prompt_tokens"],
                &["usage", "promptTokenCount"],
                &["message", "usage", "input_tokens"],
                &["message", "usage", "inputTokens"],
                &["llm_response", "usageMetadata", "promptTokenCount"],
                &["attributes", "gen_ai.usage.input_tokens"],
            ],
        ),
        output_tokens: first_u64_at(
            value,
            &[
                &["usage", "output_tokens"],
                &["usage", "outputTokens"],
                &["usage", "completion_tokens"],
                &["usage", "candidatesTokenCount"],
                &["message", "usage", "output_tokens"],
                &["message", "usage", "outputTokens"],
                &["llm_response", "usageMetadata", "candidatesTokenCount"],
                &["attributes", "gen_ai.usage.output_tokens"],
            ],
        ),
        cached_input_tokens: first_u64_at(
            value,
            &[
                &["usage", "cached_input_tokens"],
                &["usage", "cachedInputTokens"],
                &["usage", "cache_read_input_tokens"],
                &["message", "usage", "cached_input_tokens"],
                &["llm_response", "usageMetadata", "cachedContentTokenCount"],
            ],
        ),
        reasoning_tokens: first_u64_at(
            value,
            &[
                &["usage", "reasoning_output_tokens"],
                &["usage", "reasoning_tokens"],
                &["message", "usage", "reasoning_output_tokens"],
                &["llm_response", "usageMetadata", "thoughtsTokenCount"],
            ],
        ),
        context_window: first_u64_at(
            value,
            &[
                &["usage", "context_window"],
                &["usage", "contextWindow"],
                &["context_window"],
                &["contextWindow"],
            ],
        ),
    }
}

pub fn metadata_event(value: &Value) -> Option<EventKind> {
    let cwd = first_string_at(
        value,
        &[
            &["cwd"],
            &["directory"],
            &["workspace"],
            &["properties", "info", "directory"],
            &["params", "cwd"],
        ],
    )
    .map(str::to_owned);
    let model = first_string_at(
        value,
        &[
            &["model"],
            &["model_id"],
            &["modelId"],
            &["message", "model"],
            &["llm_response", "model"],
            &["properties", "info", "modelID"],
            &["properties", "info", "modelId"],
        ],
    )
    .map(str::to_owned);
    let provider = first_string_at(
        value,
        &[
            &["provider"],
            &["provider_id"],
            &["providerId"],
            &["properties", "info", "providerID"],
        ],
    )
    .map(str::to_owned);
    let pid = first_u64_at(value, &[&["pid"]]).and_then(|pid| u32::try_from(pid).ok());

    if cwd.is_none() && model.is_none() && provider.is_none() && pid.is_none() {
        None
    } else {
        Some(EventKind::Metadata {
            cwd,
            model,
            provider,
            pid,
        })
    }
}

pub fn collect_texts_at<'a>(value: &'a Value, path: &[&str], output: &mut Vec<&'a str>) {
    let Some(value) = value_at(value, path) else {
        return;
    };
    collect_text_values(value, output);
}

fn collect_text_values<'a>(value: &'a Value, output: &mut Vec<&'a str>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_text_values(value, output);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                output.push(text);
            }
            if let Some(content) = object.get("content") {
                collect_text_values(content, output);
            }
            if let Some(parts) = object.get("parts") {
                collect_text_values(parts, output);
            }
        }
        _ => {}
    }
}

pub fn tool_name(value: &Value) -> Option<String> {
    first_string_at(
        value,
        &[
            &["tool_name"],
            &["toolName"],
            &["tool"],
            &["name"],
            &["title"],
            &["properties", "tool"],
            &["properties", "part", "tool"],
            &["params", "update", "title"],
        ],
    )
    .map(str::to_owned)
}

pub fn tool_call_id(value: &Value) -> Option<String> {
    first_string_at(
        value,
        &[
            &["tool_use_id"],
            &["toolCallId"],
            &["tool_call_id"],
            &["callID"],
            &["callId"],
            &["id"],
            &["properties", "callID"],
            &["properties", "callId"],
            &["properties", "part", "callID"],
            &["params", "update", "toolCallId"],
        ],
    )
    .map(str::to_owned)
}
