use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const TELEMETRY_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolId {
    Pi,
    Droid,
    Gemini,
    Claude,
    Codex,
    OpenCode,
    Qwen,
    Kiro,
    GrokBuild,
}

impl ToolId {
    pub const ALL: [Self; 9] = [
        Self::Pi,
        Self::Droid,
        Self::Gemini,
        Self::Claude,
        Self::Codex,
        Self::OpenCode,
        Self::Qwen,
        Self::Kiro,
        Self::GrokBuild,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Droid => "droid",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Qwen => "qwen",
            Self::Kiro => "kiro",
            Self::GrokBuild => "grok-build",
        }
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ToolId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "pi" => Ok(Self::Pi),
            "droid" | "factory" | "factory-droid" => Ok(Self::Droid),
            "gemini" | "gemini-cli" => Ok(Self::Gemini),
            "claude" | "claude-code" => Ok(Self::Claude),
            "codex" | "codex-cli" => Ok(Self::Codex),
            "opencode" | "open-code" => Ok(Self::OpenCode),
            "qwen" | "qwen-code" => Ok(Self::Qwen),
            "kiro" | "kiro-cli" => Ok(Self::Kiro),
            "grok" | "grok-build" | "grok-cli" | "xai-grok" | "xai-grok-pager"
            | "xai-grok-shell" => Ok(Self::GrokBuild),
            other => Err(format!("unsupported tool: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    #[default]
    Unknown,
    Estimated,
    Derived,
    Exact,
}

impl Confidence {
    pub fn lower(self, other: Self) -> Self {
        self.min(other)
    }

    pub const fn marker(self) -> &'static str {
        match self {
            Self::Exact => "●",
            Self::Derived => "◐",
            Self::Estimated => "~",
            Self::Unknown => "-",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TelemetryEvent {
    pub schema_version: u16,
    pub id: Uuid,
    pub tool: ToolId,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub occurred_at: DateTime<Utc>,
    pub confidence: Confidence,
    pub kind: EventKind,
}

impl TelemetryEvent {
    pub fn new(
        tool: ToolId,
        session_id: impl Into<String>,
        occurred_at: DateTime<Utc>,
        confidence: Confidence,
        kind: EventKind,
    ) -> Self {
        Self {
            schema_version: TELEMETRY_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            tool,
            session_id: session_id.into(),
            turn_id: None,
            observed_at: occurred_at,
            occurred_at,
            confidence,
            kind,
        }
    }

    pub fn with_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn with_observed_at(mut self, observed_at: DateTime<Utc>) -> Self {
        self.observed_at = observed_at;
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    SessionDiscovered {
        #[serde(skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    SessionStarted,
    Metadata {
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
    },
    TurnStarted,
    FirstOutput,
    OutputDelta {
        #[serde(skip_serializing_if = "Option::is_none")]
        tokens: Option<u64>,
        characters: u64,
        bytes: u64,
    },
    ToolStarted {
        call_id: String,
        name: String,
    },
    ToolFinished {
        call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
    },
    Usage {
        #[serde(skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cached_input_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        context_window: Option<u64>,
        cumulative: bool,
    },
    WaitingForInput {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    RetryStarted,
    RetryFinished {
        #[serde(skip_serializing_if = "Option::is_none")]
        success: Option<bool>,
    },
    TurnFinished {
        success: bool,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    SessionEnded,
    Heartbeat,
}

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    New,
    Idle,
    Queue,
    Stream,
    Tool,
    Input,
    Stall,
    Retry,
    Error,
    Exited,
    #[default]
    Unknown,
}

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RateUnit {
    TokensPerSecond,
    #[default]
    Unknown,
}

impl RateUnit {
    pub const fn label(self) -> &'static str {
        match self {
            Self::TokensPerSecond => "tok/s",
            Self::Unknown => "",
        }
    }

    pub const fn compact_label(self) -> &'static str {
        match self {
            Self::TokensPerSecond => "t",
            Self::Unknown => "",
        }
    }
}

impl SessionState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::New => "NEW",
            Self::Idle => "IDLE",
            Self::Queue => "QUEUE",
            Self::Stream => "STREAM",
            Self::Tool => "TOOL",
            Self::Input => "INPUT",
            Self::Stall => "STALL",
            Self::Retry => "RETRY",
            Self::Error => "ERROR",
            Self::Exited => "EXITED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct MetricValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    pub confidence: Confidence,
}

impl MetricValue {
    pub const fn unknown() -> Self {
        Self {
            value: None,
            confidence: Confidence::Unknown,
        }
    }

    pub const fn new(value: f64, confidence: Confidence) -> Self {
        Self {
            value: Some(value),
            confidence,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionSnapshot {
    pub tool: ToolId,
    pub session_id: String,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub started_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub current_tps: MetricValue,
    pub turn_average_tps: MetricValue,
    #[serde(default)]
    pub rate_unit: RateUnit,
    pub ttft_ms: MetricValue,
    pub e2e_ms: MetricValue,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub reasoning_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    pub tool_wait_ms: i64,
    pub stall_ms: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AppSnapshot {
    pub generated_at: DateTime<Utc>,
    pub sessions: Vec<SessionSnapshot>,
    pub total_tps: f64,
    pub active_sessions: usize,
    pub generating_sessions: usize,
    pub stalled_sessions: usize,
    pub error_sessions: usize,
}

impl AppSnapshot {
    pub fn empty(generated_at: DateTime<Utc>) -> Self {
        Self {
            generated_at,
            sessions: Vec::new(),
            total_tps: 0.0,
            active_sessions: 0,
            generating_sessions: 0,
            stalled_sessions: 0,
            error_sessions: 0,
        }
    }
}
