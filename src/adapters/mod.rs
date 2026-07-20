mod claude;
mod codex;
mod common;
mod droid;
mod gemini;
mod kiro;
mod opencode;
mod pi;
mod qwen;

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{TelemetryEvent, ToolId};

pub use common::{generic_usage, ParserState, UsageFields};

#[derive(Clone, Debug)]
pub struct AdapterContext {
    pub fallback_session_id: String,
    pub observed_at: DateTime<Utc>,
    pub source_path: Option<PathBuf>,
}

impl AdapterContext {
    pub fn new(fallback_session_id: impl Into<String>, observed_at: DateTime<Utc>) -> Self {
        Self {
            fallback_session_id: fallback_session_id.into(),
            observed_at,
            source_path: None,
        }
    }

    pub fn with_source_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.source_path = Some(path.into());
        self
    }
}

pub trait Adapter: Send {
    fn tool(&self) -> ToolId;
    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent>;
}

pub fn adapter_for(tool: ToolId) -> Box<dyn Adapter> {
    match tool {
        ToolId::Pi => Box::new(pi::PiAdapter::default()),
        ToolId::Droid => Box::new(droid::DroidAdapter::default()),
        ToolId::Gemini => Box::new(gemini::GeminiAdapter::default()),
        ToolId::Claude => Box::new(claude::ClaudeAdapter::default()),
        ToolId::Codex => Box::new(codex::CodexAdapter::default()),
        ToolId::OpenCode => Box::new(opencode::OpenCodeAdapter::default()),
        ToolId::Qwen => Box::new(qwen::QwenAdapter::default()),
        ToolId::Kiro => Box::new(kiro::KiroAdapter::default()),
    }
}
