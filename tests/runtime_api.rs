use std::ffi::OsString;
use std::path::Path;

use llmeter::journal::Journal;
use llmeter::model::ToolId;
use llmeter::runtime::{
    connect_sse, ingest_file, ingest_reader, load_snapshot, snapshot_from_journal, wrap_command,
    IngestStats,
};
use tokio::io::BufReader;

#[allow(dead_code)]
async fn assert_existing_runtime_public_surface(
    path: &Path,
    journal: &Journal,
) -> anyhow::Result<()> {
    let _ = load_snapshot(path).await?;
    let _ = snapshot_from_journal(path).await?;
    let _ = ingest_file(ToolId::Codex, path, "session", journal).await?;
    let _ = ingest_reader(
        ToolId::Codex,
        BufReader::new(tokio::io::empty()),
        "session",
        None,
        journal,
    )
    .await?;
    let _ = connect_sse(
        ToolId::OpenCode,
        "http://127.0.0.1:4096/global/event",
        "session",
        journal,
    )
    .await?;
    let _ = wrap_command(
        ToolId::Codex,
        vec![OsString::from("true")],
        journal.clone(),
    )
    .await?;
    Ok(())
}

#[test]
fn ingest_stats_remains_default_constructible() {
    let _ = IngestStats::default();
}
