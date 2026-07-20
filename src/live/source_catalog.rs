use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::model::ToolId;
use crate::registry::all_tools;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredSource {
    pub tool: ToolId,
    pub path: PathBuf,
    pub modified_at: Option<SystemTime>,
}

pub fn scan_session_sources(
    home: &Path,
    max_depth: usize,
    max_per_tool: usize,
    max_total: usize,
) -> Vec<DiscoveredSource> {
    let mut all = Vec::new();

    for descriptor in all_tools() {
        let mut tool_sources = Vec::new();
        for root in descriptor.resolve_session_roots(home) {
            let _ = walk_root(
                descriptor.id,
                &root,
                0,
                max_depth,
                &mut tool_sources,
            );
        }
        tool_sources.sort_by(|left, right| {
            right
                .modified_at
                .cmp(&left.modified_at)
                .then_with(|| left.path.cmp(&right.path))
        });
        tool_sources.truncate(max_per_tool);
        all.extend(tool_sources);
    }

    all.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.tool.cmp(&right.tool))
            .then_with(|| left.path.cmp(&right.path))
    });
    all.truncate(max_total);
    all
}

fn walk_root(
    tool: ToolId,
    path: &Path,
    depth: usize,
    max_depth: usize,
    output: &mut Vec<DiscoveredSource>,
) -> io::Result<()> {
    if depth > max_depth || !path.exists() {
        return Ok(());
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound) => {
            return Ok(())
        }
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        if matches_session_source(tool, path) {
            output.push(DiscoveredSource {
                tool,
                path: path.to_path_buf(),
                modified_at: metadata.modified().ok(),
            });
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound) => {
            return Ok(())
        }
        Err(error) => return Err(error),
    };
    for entry in entries.flatten() {
        walk_root(tool, &entry.path(), depth + 1, max_depth, output)?;
    }
    Ok(())
}

pub fn matches_session_source(tool: ToolId, path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let file_name = file_name.to_ascii_lowercase();
    let is_jsonl = path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"));

    match tool {
        ToolId::Pi | ToolId::Claude | ToolId::Qwen | ToolId::Kiro => is_jsonl,
        ToolId::Codex => {
            is_jsonl && file_name.starts_with("rollout-")
        }
        ToolId::GrokBuild => file_name == "updates.jsonl",
        ToolId::Droid => {
            is_jsonl && contains_any(&file_name, &["session", "event", "stream"])
        }
        ToolId::Gemini => {
            is_jsonl && contains_any(&file_name, &["session", "event", "telemetry"])
        }
        ToolId::OpenCode => {
            is_jsonl && contains_any(&file_name, &["session", "event", "message"])
        }
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn tool_specific_matchers_exclude_unrelated_json_and_accept_native_sources() {
        let temp = tempdir().unwrap();
        let home = temp.path();

        let codex = home.join(".codex/sessions/2026/07/20");
        let grok = home.join(".grok/sessions/project/session-1");
        let claude = home.join(".claude/projects/project");
        let gemini = home.join(".gemini");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir_all(&grok).unwrap();
        fs::create_dir_all(&claude).unwrap();
        fs::create_dir_all(&gemini).unwrap();

        fs::write(codex.join("rollout-a.jsonl"), "{}\n").unwrap();
        fs::write(codex.join("config.json"), "{}").unwrap();
        fs::write(codex.join("random.jsonl"), "{}\n").unwrap();
        fs::write(grok.join("updates.jsonl"), "{}\n").unwrap();
        fs::write(grok.join("feedback.jsonl"), "{}\n").unwrap();
        fs::write(claude.join("conversation.jsonl"), "{}\n").unwrap();
        fs::write(gemini.join("settings.json"), "{}").unwrap();
        fs::write(gemini.join("telemetry-events.jsonl"), "{}\n").unwrap();

        let sources = scan_session_sources(home, 8, 16, 64);
        let paths = sources
            .iter()
            .map(|source| source.path.strip_prefix(home).unwrap().to_path_buf())
            .collect::<Vec<_>>();

        assert!(paths.contains(&PathBuf::from(".codex/sessions/2026/07/20/rollout-a.jsonl")));
        assert!(paths.contains(&PathBuf::from(".grok/sessions/project/session-1/updates.jsonl")));
        assert!(paths.contains(&PathBuf::from(".claude/projects/project/conversation.jsonl")));
        assert!(paths.contains(&PathBuf::from(".gemini/telemetry-events.jsonl")));
        assert!(!paths.iter().any(|path| path.ends_with("config.json")));
        assert!(!paths.iter().any(|path| path.ends_with("random.jsonl")));
        assert!(!paths.iter().any(|path| path.ends_with("feedback.jsonl")));
        assert!(!paths.iter().any(|path| path.ends_with("settings.json")));
    }

    #[cfg(unix)]
    #[test]
    fn source_scan_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        let outside = temp.path().join("outside");
        fs::create_dir_all(home.join(".codex/sessions")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("rollout-secret.jsonl"), "{}\n").unwrap();
        symlink(&outside, home.join(".codex/sessions/linked")).unwrap();

        let sources = scan_session_sources(&home, 8, 16, 64);
        assert!(sources.is_empty());
    }
}
