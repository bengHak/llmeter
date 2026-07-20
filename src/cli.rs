use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokio::io::BufReader;

use crate::discovery::{detect_processes, discover_session_files};
use crate::journal::Journal;
use crate::model::{AppSnapshot, ToolId};
use crate::registry::all_tools;
use crate::runtime::{
    connect_sse, ingest_file, ingest_reader, load_snapshot, snapshot_from_journal, wrap_command,
};
use crate::setup::setup_snippet;
use crate::tui;

#[derive(Debug, Parser)]
#[command(
    name = "llmeter",
    version,
    about = "Measure live LLM coding-agent sessions in a terminal",
    arg_required_else_help = false
)]
pub struct Cli {
    #[arg(long, global = true, env = "LLMETER_DATA_DIR", value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the live terminal dashboard (default).
    Tui,
    /// Print one human-readable snapshot.
    Once,
    /// Print one JSON snapshot.
    Json,
    /// Parse JSONL/SSE records from a file or stdin into the normalized journal.
    Ingest {
        #[arg(long)]
        tool: ToolId,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Receive one command-hook JSON payload from stdin.
    Hook {
        #[arg(long)]
        tool: ToolId,
    },
    /// Connect to a tool's Server-Sent Events endpoint.
    Connect {
        #[arg(long, default_value = "opencode")]
        tool: ToolId,
        #[arg(long, default_value = "http://127.0.0.1:4096/global/event")]
        url: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Proxy a JSONL/JSON-RPC child process while measuring its stream.
    Wrap {
        #[arg(long)]
        tool: ToolId,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Replay a normalized journal.
    Replay {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Show executable, process, session-root, and journal diagnostics.
    Doctor,
    /// Print a safe integration snippet for a supported tool.
    Setup {
        tool: ToolId,
        #[arg(long)]
        binary: Option<PathBuf>,
    },
}

pub async fn run() -> Result<i32> {
    let cli = Cli::parse();
    let data_dir = resolve_data_dir(cli.data_dir);
    tokio::fs::create_dir_all(&data_dir)
        .await
        .with_context(|| format!("create data directory {}", data_dir.display()))?;
    let journal = Journal::new(data_dir.join("events.jsonl"));

    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => {
            tui::run_live(&data_dir).await?;
            Ok(0)
        }
        Command::Once => {
            print_human_snapshot(&load_snapshot(&data_dir).await?);
            Ok(0)
        }
        Command::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&load_snapshot(&data_dir).await?)?
            );
            Ok(0)
        }
        Command::Ingest {
            tool,
            file,
            session,
        } => {
            let fallback = session.unwrap_or_else(|| {
                file.as_deref()
                    .and_then(Path::file_stem)
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{}-stdin", tool.as_str()))
            });
            let stats = if let Some(file) = file {
                ingest_file(tool, &file, fallback, &journal).await?
            } else {
                ingest_reader(
                    tool,
                    BufReader::new(tokio::io::stdin()),
                    fallback,
                    None,
                    &journal,
                )
                .await?
            };
            eprintln!(
                "ingested {} records, emitted {} events, skipped {} malformed records",
                stats.records, stats.events, stats.malformed_records
            );
            Ok(0)
        }
        Command::Hook { tool } => {
            let stats = ingest_reader(
                tool,
                BufReader::new(tokio::io::stdin()),
                format!("{}-hook", tool.as_str()),
                None,
                &journal,
            )
            .await?;
            if stats.malformed_records > 0 {
                eprintln!("llmeter ignored malformed hook input");
            }
            println!("{{}}");
            Ok(0)
        }
        Command::Connect { tool, url, session } => {
            let fallback = session.unwrap_or_else(|| format!("{}-sse", tool.as_str()));
            let stats = connect_sse(tool, &url, fallback, &journal).await?;
            eprintln!(
                "SSE ended after {} records and {} normalized events",
                stats.records, stats.events
            );
            Ok(0)
        }
        Command::Wrap { tool, command } => wrap_command(tool, command, journal).await,
        Command::Replay { file, json } => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshot_from_journal(&file).await?)?
                );
            } else {
                tui::run_replay(&file).await?;
            }
            Ok(0)
        }
        Command::Doctor => {
            print_doctor(&data_dir)?;
            Ok(0)
        }
        Command::Setup { tool, binary } => {
            let binary = binary
                .or_else(|| std::env::current_exe().ok())
                .unwrap_or_else(|| PathBuf::from("llmeter"));
            println!("{}", setup_snippet(tool, &binary.to_string_lossy())?);
            Ok(0)
        }
    }
}

fn resolve_data_dir(explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("llmeter")
    })
}

fn print_human_snapshot(snapshot: &AppSnapshot) {
    println!(
        "llmeter: {} sessions, {} active, {:.1} tok/s",
        snapshot.sessions.len(),
        snapshot.active_sessions,
        snapshot.total_tps,
    );
    println!(
        "{:<8} {:<10} {:<24} {:>9} {:>9} {:>8}",
        "STATE", "TOOL", "MODEL", "NOW(t/s)", "TTFT(ms)", "OUT(tok)"
    );
    for session in &snapshot.sessions {
        let now = session.current_tps.value.map_or_else(
            || "-".to_owned(),
            |value| {
                format!(
                    "{}{value:.1}{}",
                    session.current_tps.confidence.marker(),
                    session.rate_unit.compact_label(),
                )
            },
        );
        let ttft = session.ttft_ms.value.map_or_else(
            || "-".to_owned(),
            |value| format!("{}{value:.0}", session.ttft_ms.confidence.marker()),
        );
        println!(
            "{:<8} {:<10} {:<24} {:>9} {:>9} {:>8}",
            session.state.label(),
            session.tool,
            session.model.as_deref().unwrap_or("-"),
            now,
            ttft,
            session.output_tokens,
        );
    }
}

fn print_doctor(data_dir: &Path) -> Result<()> {
    let processes = detect_processes().unwrap_or_default();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let files = discover_session_files(&home, 6, 256).unwrap_or_default();

    println!("llmeter doctor");
    println!("data directory: {}", data_dir.display());
    println!(
        "journal: {} ({})",
        data_dir.join("events.jsonl").display(),
        if data_dir.join("events.jsonl").exists() {
            "present"
        } else {
            "not created"
        }
    );
    println!();
    println!(
        "{:<10} {:<11} {:>9} {:>13}  TRANSPORTS",
        "TOOL", "EXECUTABLE", "PROCESSES", "SESSION FILES"
    );
    for descriptor in all_tools() {
        let executable = descriptor
            .executables
            .iter()
            .any(|candidate| executable_on_path(candidate));
        let process_count = processes
            .iter()
            .filter(|process| process.tool == Some(descriptor.id))
            .count();
        let file_count = files
            .iter()
            .filter(|file| file.tool == descriptor.id)
            .count();
        println!(
            "{:<10} {:<11} {:>9} {:>13}  {:?}",
            descriptor.id,
            if executable { "found" } else { "missing" },
            process_count,
            file_count,
            descriptor.transports,
        );
    }
    Ok(())
}

fn executable_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            for extension in ["exe", "cmd", "bat"] {
                if directory.join(format!("{name}.{extension}")).is_file() {
                    return true;
                }
            }
        }
        false
    })
}
