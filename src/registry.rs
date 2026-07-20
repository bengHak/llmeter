use std::path::{Path, PathBuf};

use crate::model::ToolId;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Transport {
    Process,
    SessionFile,
    JsonLines,
    JsonRpc,
    Hook,
    OpenTelemetry,
    Sse,
    Acp,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    Discovery,
    SessionLifecycle,
    TurnBoundary,
    OutputDelta,
    ToolSpan,
    Usage,
    ContextWindow,
    WaitingState,
    RetryState,
    ModelMetadata,
    TerminalLocation,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolDescriptor {
    pub id: ToolId,
    pub display_name: &'static str,
    pub executables: &'static [&'static str],
    pub process_markers: &'static [&'static str],
    pub session_roots: &'static [&'static str],
    pub transports: &'static [Transport],
    pub capabilities: &'static [Capability],
}

impl ToolDescriptor {
    pub fn resolve_session_roots(&self, home: &Path) -> Vec<PathBuf> {
        self.session_roots
            .iter()
            .map(|root| {
                root.strip_prefix("~/")
                    .map_or_else(|| PathBuf::from(root), |suffix| home.join(suffix))
            })
            .collect()
    }

    pub fn matches_command(&self, command: &str) -> bool {
        let normalized = command.to_ascii_lowercase();
        self.executables.iter().any(|executable| {
            command_tokens(&normalized).any(|token| {
                Path::new(token)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == *executable)
            })
        }) || self
            .process_markers
            .iter()
            .any(|marker| normalized.contains(marker))
    }
}

fn command_tokens(command: &str) -> impl Iterator<Item = &str> {
    command.split_whitespace().map(|token| {
        token
            .trim_matches(|character: char| character == '\'' || character == '"')
            .trim_end_matches([';', ','])
    })
}

const PI_TRANSPORTS: &[Transport] = &[
    Transport::Process,
    Transport::SessionFile,
    Transport::JsonLines,
    Transport::JsonRpc,
];
const DROID_TRANSPORTS: &[Transport] = &[
    Transport::Process,
    Transport::JsonLines,
    Transport::JsonRpc,
    Transport::Hook,
];
const GEMINI_TRANSPORTS: &[Transport] = &[
    Transport::Process,
    Transport::Hook,
    Transport::OpenTelemetry,
];
const CLAUDE_TRANSPORTS: &[Transport] =
    &[Transport::Process, Transport::SessionFile, Transport::Hook];
const CODEX_TRANSPORTS: &[Transport] = &[
    Transport::Process,
    Transport::SessionFile,
    Transport::JsonLines,
    Transport::Hook,
];
const OPENCODE_TRANSPORTS: &[Transport] = &[
    Transport::Process,
    Transport::SessionFile,
    Transport::JsonLines,
    Transport::Sse,
    Transport::Acp,
];
const QWEN_TRANSPORTS: &[Transport] = &[
    Transport::Process,
    Transport::SessionFile,
    Transport::JsonLines,
];
const KIRO_TRANSPORTS: &[Transport] = &[
    Transport::Process,
    Transport::SessionFile,
    Transport::Hook,
    Transport::Acp,
];

const PI_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::OutputDelta,
    Capability::ToolSpan,
    Capability::Usage,
    Capability::ContextWindow,
    Capability::RetryState,
    Capability::ModelMetadata,
];
const DROID_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::OutputDelta,
    Capability::ToolSpan,
    Capability::WaitingState,
    Capability::ModelMetadata,
];
const GEMINI_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::OutputDelta,
    Capability::ToolSpan,
    Capability::Usage,
    Capability::ContextWindow,
    Capability::ModelMetadata,
];
const CLAUDE_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::ToolSpan,
    Capability::WaitingState,
    Capability::ModelMetadata,
];
const CODEX_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::Usage,
    Capability::ContextWindow,
    Capability::ToolSpan,
    Capability::ModelMetadata,
];
const OPENCODE_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::OutputDelta,
    Capability::ToolSpan,
    Capability::WaitingState,
    Capability::ModelMetadata,
];
const QWEN_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::OutputDelta,
    Capability::ToolSpan,
    Capability::Usage,
    Capability::ModelMetadata,
];
const KIRO_CAPABILITIES: &[Capability] = &[
    Capability::Discovery,
    Capability::SessionLifecycle,
    Capability::TurnBoundary,
    Capability::OutputDelta,
    Capability::ToolSpan,
    Capability::WaitingState,
    Capability::ModelMetadata,
];

const TOOLS: &[ToolDescriptor] = &[
    ToolDescriptor {
        id: ToolId::Pi,
        display_name: "Pi",
        executables: &["pi"],
        process_markers: &["pi-coding-agent"],
        session_roots: &["~/.pi/agent/sessions"],
        transports: PI_TRANSPORTS,
        capabilities: PI_CAPABILITIES,
    },
    ToolDescriptor {
        id: ToolId::Droid,
        display_name: "Factory Droid",
        executables: &["droid"],
        process_markers: &["factory-droid"],
        session_roots: &["~/.factory"],
        transports: DROID_TRANSPORTS,
        capabilities: DROID_CAPABILITIES,
    },
    ToolDescriptor {
        id: ToolId::Gemini,
        display_name: "Gemini CLI",
        executables: &["gemini"],
        process_markers: &["@google/gemini-cli", "gemini-cli"],
        session_roots: &["~/.gemini"],
        transports: GEMINI_TRANSPORTS,
        capabilities: GEMINI_CAPABILITIES,
    },
    ToolDescriptor {
        id: ToolId::Claude,
        display_name: "Claude Code",
        executables: &["claude"],
        process_markers: &["@anthropic-ai/claude-code", "claude-code"],
        session_roots: &["~/.claude/projects"],
        transports: CLAUDE_TRANSPORTS,
        capabilities: CLAUDE_CAPABILITIES,
    },
    ToolDescriptor {
        id: ToolId::Codex,
        display_name: "Codex CLI",
        executables: &["codex"],
        process_markers: &["@openai/codex", "codex-cli"],
        session_roots: &["~/.codex/sessions"],
        transports: CODEX_TRANSPORTS,
        capabilities: CODEX_CAPABILITIES,
    },
    ToolDescriptor {
        id: ToolId::OpenCode,
        display_name: "OpenCode",
        executables: &["opencode"],
        process_markers: &["anomalyco/opencode", "sst/opencode"],
        session_roots: &["~/.local/share/opencode", "~/.opencode"],
        transports: OPENCODE_TRANSPORTS,
        capabilities: OPENCODE_CAPABILITIES,
    },
    ToolDescriptor {
        id: ToolId::Qwen,
        display_name: "Qwen Code",
        executables: &["qwen"],
        process_markers: &["@qwen-code/qwen-code", "qwen-code"],
        session_roots: &["~/.qwen/projects"],
        transports: QWEN_TRANSPORTS,
        capabilities: QWEN_CAPABILITIES,
    },
    ToolDescriptor {
        id: ToolId::Kiro,
        display_name: "Kiro CLI",
        executables: &["kiro-cli", "kiro"],
        process_markers: &["kiro-cli"],
        session_roots: &["~/.kiro/sessions/cli"],
        transports: KIRO_TRANSPORTS,
        capabilities: KIRO_CAPABILITIES,
    },
];

pub fn all_tools() -> &'static [ToolDescriptor] {
    TOOLS
}

pub fn descriptor(id: ToolId) -> &'static ToolDescriptor {
    TOOLS
        .iter()
        .find(|descriptor| descriptor.id == id)
        .expect("all ToolId variants have descriptors")
}
