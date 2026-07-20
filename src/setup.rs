use anyhow::{bail, Result};

use crate::model::ToolId;

pub fn setup_snippet(tool: ToolId, binary: &str) -> Result<String> {
    if binary.trim().is_empty() {
        bail!("binary path cannot be empty");
    }
    let command = format!("{binary} hook --tool {}", tool.as_str());
    let snippet = match tool {
        ToolId::Claude => command_hook_json(
            &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
                "PermissionRequest",
                "Stop",
                "SessionEnd",
            ],
            &command,
        ),
        ToolId::Droid => command_hook_json(
            &[
                "SessionStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "Notification",
                "Stop",
                "SessionEnd",
            ],
            &command,
        ),
        ToolId::Gemini => command_hook_json(
            &[
                "SessionStart",
                "BeforeAgent",
                "AfterModel",
                "BeforeTool",
                "AfterTool",
                "AfterAgent",
                "Notification",
                "SessionEnd",
            ],
            &command,
        ),
        ToolId::Kiro => format!(
            "# Add command hooks for agentSpawn, userPromptSubmit, preToolUse, postToolUse, stop\n# Hook command:\n{command}\n\n# ACP alternative:\n{binary} wrap --tool kiro -- kiro-cli acp"
        ),
        ToolId::Pi => format!(
            "{binary} wrap --tool pi -- pi --mode rpc\n# Or ingest a saved session:\n{binary} ingest --tool pi --file ~/.pi/agent/sessions/<session>.jsonl"
        ),
        ToolId::Codex => format!(
            "{binary} wrap --tool codex -- codex exec --json <prompt>\n# Interactive sessions are also discovered under ~/.codex/sessions."
        ),
        ToolId::OpenCode => format!(
            "opencode serve --hostname 127.0.0.1 --port 4096\n{binary} connect --tool opencode --url http://127.0.0.1:4096/global/event"
        ),
        ToolId::Qwen => format!(
            "{binary} wrap --tool qwen -- qwen --output-format stream-json --include-partial-messages -p <prompt>"
        ),
        ToolId::GrokBuild => {
            let hooks = command_hook_json_without_matcher(
                &[
                    "SessionStart",
                    "UserPromptSubmit",
                    "PreToolUse",
                    "PostToolUse",
                    "PostToolUseFailure",
                    "PermissionDenied",
                    "Stop",
                    "StopFailure",
                    "Notification",
                    "SubagentStart",
                    "SubagentStop",
                    "SessionEnd",
                ],
                &command,
            );
            format!(
                "# Save this hook configuration as ~/.grok/hooks/llmeter.json\n{hooks}\n\n# Exact headless streaming path:\n{binary} wrap --tool grok-build -- grok --no-auto-update -p <prompt> --output-format streaming-json\n\n# ACP path:\n{binary} wrap --tool grok-build -- grok --no-auto-update agent stdio\n\n# Existing interactive sessions are discovered under ~/.grok/sessions/**/updates.jsonl."
            )
        }
    };
    Ok(snippet)
}

fn command_hook_json(events: &[&str], command: &str) -> String {
    let mut hooks = serde_json::Map::new();
    for event in events {
        hooks.insert(
            (*event).to_owned(),
            serde_json::json!([{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": command,
                    "timeout": 5
                }]
            }]),
        );
    }
    serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks }))
        .expect("hook snippet is serializable")
}

fn command_hook_json_without_matcher(events: &[&str], command: &str) -> String {
    let mut hooks = serde_json::Map::new();
    for event in events {
        hooks.insert(
            (*event).to_owned(),
            serde_json::json!([{
                "hooks": [{
                    "type": "command",
                    "command": command,
                    "timeout": 5
                }]
            }]),
        );
    }
    serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks }))
        .expect("hook snippet is serializable")
}
