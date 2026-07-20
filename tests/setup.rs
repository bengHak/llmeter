use llmeter::model::ToolId;
use llmeter::setup::setup_snippet;

#[test]
fn command_hook_snippets_pipe_stdin_to_privacy_safe_hook_command() {
    for tool in [
        ToolId::Claude,
        ToolId::Droid,
        ToolId::Gemini,
        ToolId::Kiro,
        ToolId::GrokBuild,
    ] {
        let snippet = setup_snippet(tool, "/usr/local/bin/llmeter").unwrap();
        assert!(snippet.contains("llmeter"));
        assert!(snippet.contains("hook"));
        assert!(snippet.contains(tool.as_str()));
        assert!(!snippet.contains("logPrompts\": true"));
        assert!(!snippet.contains("store_content"));
    }
}

#[test]
fn grok_setup_uses_native_hook_shape_and_documents_all_ingest_paths() {
    let snippet = setup_snippet(ToolId::GrokBuild, "/usr/local/bin/llmeter").unwrap();

    assert!(snippet.contains("~/.grok/hooks/llmeter.json"));
    assert!(snippet.contains("StopFailure"));
    assert!(snippet.contains("PermissionDenied"));
    assert!(snippet.contains("--output-format streaming-json"));
    assert!(snippet.contains("grok --no-auto-update agent stdio"));
    assert!(snippet.contains("~/.grok/sessions/**/updates.jsonl"));
    assert!(!snippet.contains("\"matcher\": \"*\""));
}
