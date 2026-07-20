use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Duration, Utc};

use crate::model::{
    AppSnapshot, Confidence, EventKind, MetricValue, RateUnit, SessionSnapshot, SessionState,
    TelemetryEvent, ToolId,
};

#[derive(Clone, Debug)]
pub struct AggregatorConfig {
    pub rate_window: Duration,
    pub stall_threshold: Duration,
    pub exited_session_ttl: Duration,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self {
            rate_window: Duration::seconds(2),
            stall_threshold: Duration::seconds(2),
            exited_session_ttl: Duration::seconds(30),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SessionKey {
    tool: ToolId,
    session_id: String,
}

#[derive(Clone, Debug)]
struct RateSample {
    at: DateTime<Utc>,
    units: f64,
    confidence: Confidence,
}

#[derive(Clone, Debug)]
struct ActiveTool;

#[derive(Clone, Debug)]
struct TurnRuntime {
    id: Option<String>,
    started_at: DateTime<Utc>,
    start_confidence: Confidence,
    first_output_at: Option<DateTime<Utc>>,
    first_output_confidence: Confidence,
    last_output_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    finish_confidence: Confidence,
    samples: VecDeque<RateSample>,
    token_output_units: f64,
    token_output_confidence: Confidence,
    exact_delta_tokens: u64,
    reported_input_tokens: u64,
    reported_output_tokens: u64,
    reported_cached_input_tokens: u64,
    reported_reasoning_tokens: u64,
    active_tools: HashMap<String, ActiveTool>,
    tool_block_started_at: Option<DateTime<Utc>>,
    tool_wait: Duration,
    tool_wait_since_last_output: Duration,
    waiting_for_input: bool,
    retrying: bool,
    failed: bool,
}

impl TurnRuntime {
    fn new(id: Option<String>, started_at: DateTime<Utc>, confidence: Confidence) -> Self {
        Self {
            id,
            started_at,
            start_confidence: confidence,
            first_output_at: None,
            first_output_confidence: Confidence::Unknown,
            last_output_at: None,
            finished_at: None,
            finish_confidence: Confidence::Unknown,
            samples: VecDeque::new(),
            token_output_units: 0.0,
            token_output_confidence: Confidence::Unknown,
            exact_delta_tokens: 0,
            reported_input_tokens: 0,
            reported_output_tokens: 0,
            reported_cached_input_tokens: 0,
            reported_reasoning_tokens: 0,
            active_tools: HashMap::new(),
            tool_block_started_at: None,
            tool_wait: Duration::zero(),
            tool_wait_since_last_output: Duration::zero(),
            waiting_for_input: false,
            retrying: false,
            failed: false,
        }
    }

    fn record_output(
        &mut self,
        at: DateTime<Utc>,
        tokens: Option<u64>,
        confidence: Confidence,
    ) {
        // Timing (TTFT / stall) uses any output event; rates only use token counts.
        let output_confidence = if tokens.is_some() {
            confidence
        } else {
            Confidence::Estimated
        };
        if self.first_output_at.is_none() {
            self.first_output_at = Some(at);
            self.first_output_confidence = output_confidence;
        }
        self.last_output_at = Some(at);
        self.waiting_for_input = false;
        self.retrying = false;
        self.tool_wait_since_last_output = Duration::zero();

        let Some(tokens) = tokens else {
            return;
        };
        let units = tokens as f64;
        self.exact_delta_tokens = self.exact_delta_tokens.saturating_add(tokens);
        self.samples.push_back(RateSample {
            at,
            units,
            confidence,
        });
        self.token_output_units += units;
        self.token_output_confidence =
            lower_known_confidence(self.token_output_confidence, confidence);
    }

    fn finish_tool(&mut self, call_id: &str, at: DateTime<Utc>) {
        if self.active_tools.remove(call_id).is_none() {
            return;
        }
        if !self.active_tools.is_empty() {
            return;
        }
        self.finish_tool_block(at);
    }

    fn finish_tool_block(&mut self, at: DateTime<Utc>) {
        let Some(started_at) = self.tool_block_started_at.take() else {
            return;
        };
        self.tool_wait += non_negative_duration(at - started_at);
        if let Some(last_output) = self.last_output_at {
            let overlap_start = started_at.max(last_output);
            self.tool_wait_since_last_output += non_negative_duration(at - overlap_start);
        }
    }

    fn accounted_output_tokens(&self) -> u64 {
        self.exact_delta_tokens.max(self.reported_output_tokens)
    }

}
