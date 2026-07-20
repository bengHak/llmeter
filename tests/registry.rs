use std::collections::HashSet;
use std::path::Path;

use llmeter::discovery::parse_ps_line;
use llmeter::model::ToolId;
use llmeter::registry::{all_tools, Capability, Transport};

#[test]
fn registry_contains_all_phase_one_and_two_tools_once() {
    let tools = all_tools();
    let ids: HashSet<_> = tools.iter().map(|tool| tool.id).collect();

    assert_eq!(tools.len(), 8);
    assert_eq!(ids.len(), 8);
    for id in [
        ToolId::Pi,
        ToolId::Droid,
        ToolId::Gemini,
        ToolId::Claude,
        ToolId::Codex,
        ToolId::OpenCode,
        ToolId::Qwen,
        ToolId::Kiro,
    ] {
        assert!(ids.contains(&id), "missing {id}");
    }
}

#[test]
fn descriptors_expose_expected_native_transports_and_capabilities() {
    let tools = all_tools();
    let pi = tools.iter().find(|tool| tool.id == ToolId::Pi).unwrap();
    let gemini = tools.iter().find(|tool| tool.id == ToolId::Gemini).unwrap();
    let opencode = tools
        .iter()
        .find(|tool| tool.id == ToolId::OpenCode)
        .unwrap();
    let kiro = tools.iter().find(|tool| tool.id == ToolId::Kiro).unwrap();

    assert!(pi.transports.contains(&Transport::JsonRpc));
    assert!(pi.capabilities.contains(&Capability::OutputDelta));
    assert!(gemini.transports.contains(&Transport::Hook));
    assert!(gemini.transports.contains(&Transport::OpenTelemetry));
    assert!(opencode.transports.contains(&Transport::Sse));
    assert!(kiro.transports.contains(&Transport::Acp));
}

#[test]
fn session_roots_expand_against_an_explicit_home() {
    let tools = all_tools();
    let home = Path::new("/home/example");
    let codex = tools.iter().find(|tool| tool.id == ToolId::Codex).unwrap();
    let roots = codex.resolve_session_roots(home);

    assert!(roots
        .iter()
        .any(|path| path == Path::new("/home/example/.codex/sessions")));
}

#[test]
fn parses_ps_rows_without_losing_command_arguments() {
    let row = " 4242  120  91 codex --model gpt-example --search";
    let process = parse_ps_line(row).expect("process row");

    assert_eq!(process.pid, 4242);
    assert_eq!(process.parent_pid, Some(120));
    assert_eq!(process.elapsed_secs, Some(91));
    assert_eq!(process.command, "codex --model gpt-example --search");
}
