use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use crate::model::ToolId;
use crate::registry::all_tools;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub elapsed_secs: Option<u64>,
    pub command: String,
    pub tool: Option<ToolId>,
}

pub fn parse_ps_line(line: &str) -> Option<ProcessInfo> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse().ok()?;
    let parent_pid = fields.next().and_then(|value| value.parse().ok());
    let elapsed_secs = fields.next().and_then(parse_elapsed_secs);
    let command = fields.collect::<Vec<_>>().join(" ");
    if command.is_empty() {
        return None;
    }
    let tool = all_tools()
        .iter()
        .find(|descriptor| descriptor.matches_command(&command))
        .map(|descriptor| descriptor.id);
    Some(ProcessInfo {
        pid,
        parent_pid,
        elapsed_secs,
        command,
        tool,
    })
}

fn parse_elapsed_secs(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.parse() {
        return Some(seconds);
    }
    let (days, clock) = if let Some((days, clock)) = value.split_once('-') {
        (days.parse::<u64>().ok()?, clock)
    } else {
        (0_u64, value)
    };
    let parts = clock
        .split(':')
        .map(str::parse)
        .collect::<Result<Vec<u64>, _>>()
        .ok()?;
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    days.checked_mul(86_400)?
        .checked_add(hours.checked_mul(3_600)?)?
        .checked_add(minutes.checked_mul(60)?)?
        .checked_add(seconds)
}

pub fn detect_processes() -> io::Result<Vec<ProcessInfo>> {
    let output = run_ps(&["-eo", "pid=,ppid=,etimes=,args="])
        .or_else(|_| run_ps(&["-axo", "pid=,ppid=,etime=,command="]));
    let output = match output {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_ps_line)
        .filter(|process| process.tool.is_some())
        .collect())
}

fn run_ps(arguments: &[&str]) -> io::Result<std::process::Output> {
    let output = Command::new("ps").args(arguments).output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(io::Error::other(format!(
            "ps exited with {}",
            output.status
        )))
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveredSessionFile {
    pub tool: ToolId,
    pub path: PathBuf,
    pub modified_at: Option<SystemTime>,
}

pub fn discover_session_files(
    home: &Path,
    max_depth: usize,
    max_files: usize,
) -> io::Result<Vec<DiscoveredSessionFile>> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for descriptor in all_tools() {
        for root in descriptor.resolve_session_roots(home) {
            walk_session_root(
                descriptor.id,
                &root,
                0,
                max_depth,
                max_files,
                &mut seen,
                &mut results,
            )?;
            if results.len() >= max_files {
                break;
            }
        }
        if results.len() >= max_files {
            break;
        }
    }

    results.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
    Ok(results)
}

fn walk_session_root(
    tool: ToolId,
    path: &Path,
    depth: usize,
    max_depth: usize,
    max_files: usize,
    seen: &mut HashSet<PathBuf>,
    results: &mut Vec<DiscoveredSessionFile>,
) -> io::Result<()> {
    if results.len() >= max_files || depth > max_depth || !path.exists() {
        return Ok(());
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        if is_session_file(path) {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            if seen.insert(canonical) {
                results.push(DiscoveredSessionFile {
                    tool,
                    path: path.to_path_buf(),
                    modified_at: metadata.modified().ok(),
                });
            }
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries.flatten() {
        walk_session_root(
            tool,
            &entry.path(),
            depth + 1,
            max_depth,
            max_files,
            seen,
            results,
        )?;
        if results.len() >= max_files {
            break;
        }
    }
    Ok(())
}

fn is_session_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("json") | Some("jsonl") | Some("ndjson")
    )
}
