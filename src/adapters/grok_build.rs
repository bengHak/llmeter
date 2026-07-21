use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::adapters::common::{
    event, first_string_at, first_u64_at, safe_output_delta, string_at, turn_id, ParserState,
    UsageFields,
};
use crate::adapters::{Adapter, AdapterContext};
use crate::model::{Confidence, EventKind, TelemetryEvent, ToolId};

#[derive(Default)]
pub struct GrokBuildAdapter {
    state: ParserState,
    pending_new_sessions: HashMap<String, String>,
    pending_prompts: HashMap<String, String>,
    retrying_sessions: HashSet<String>,
    unified_roots: HashMap<u32, String>,
    unified_usage: HashMap<String, UsageFields>,
}

impl Adapter for GrokBuildAdapter {
    fn tool(&self) -> ToolId {
        ToolId::GrokBuild
    }

    fn parse_record(&mut self, value: &Value, context: &AdapterContext) -> Vec<TelemetryEvent> {
        if hook_name(value).is_some() {
            return self.parse_hook(value, context);
        }
        if value.get("msg").is_some() && value.get("pid").is_some() {
            return self.parse_unified(value, context);
        }
        if is_headless_record(value) {
            return self.parse_headless(value, context);
        }
        if value.get("jsonrpc").is_some() || value.get("method").is_some() {
            return self.parse_rpc(value, context);
        }
        Vec::new()
    }
}

include!("grok_build/parse_hook.rs");
include!("grok_build/parse_headless.rs");
include!("grok_build/parse_unified.rs");
include!("grok_build/parse_rpc.rs");
include!("grok_build/parse_rpc_response.rs");
include!("grok_build/parse_session_update.rs");
include!("grok_build/helpers_core.rs");
include!("grok_build/helpers_usage.rs");
