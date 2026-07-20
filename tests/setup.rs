use llmeter::model::ToolId;
use llmeter::setup::setup_snippet;

#[test]
fn command_hook_snippets_pipe_stdin_to_privacy_safe_hook_command() {
    for tool in [ToolId::Claude, ToolId::Droid, ToolId::Gemini, ToolId::Kiro] {
        let snippet = setup_snippet(tool, "/usr/local/bin/llmeter").unwrap();
        assert!(snippet.contains("llmeter"));
        assert!(snippet.contains("hook"));
        assert!(snippet.contains(tool.as_str()));
        assert!(!snippet.contains("logPrompts\": true"));
        assert!(!snippet.contains("store_content"));
    }
}
