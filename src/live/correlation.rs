use std::collections::{HashMap, HashSet};

use crate::model::{AppSnapshot, RateUnit, SessionSnapshot, SessionState, ToolId};

pub fn correlate_process_sessions(mut snapshot: AppSnapshot) -> AppSnapshot {
    let mut removed = HashSet::new();

    for process_index in process_indices(&snapshot.sessions) {
        let Some(pid) = snapshot.sessions[process_index].pid else {
            continue;
        };
        let tool = snapshot.sessions[process_index].tool;
        let candidates = snapshot
            .sessions
            .iter()
            .enumerate()
            .filter(|(index, session)| {
                !removed.contains(index)
                    && !is_process_only(session)
                    && session.state != SessionState::Exited
                    && session.tool == tool
                    && session.pid == Some(pid)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            merge_process_into_native(&mut snapshot.sessions, process_index, candidates[0]);
            removed.insert(process_index);
        }
    }

    let mut remaining_by_tool: HashMap<ToolId, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (index, session) in snapshot.sessions.iter().enumerate() {
        if removed.contains(&index) {
            continue;
        }
        let entry = remaining_by_tool.entry(session.tool).or_default();
        if is_process_only(session) {
            if session.state != SessionState::Exited {
                entry.0.push(index);
            }
        } else if session.state != SessionState::Exited && session.pid.is_none() {
            entry.1.push(index);
        }
    }
    for (processes, natives) in remaining_by_tool.into_values() {
        if processes.len() == 1 && natives.len() == 1 {
            merge_process_into_native(&mut snapshot.sessions, processes[0], natives[0]);
            removed.insert(processes[0]);
        }
    }

    snapshot.sessions = snapshot
        .sessions
        .into_iter()
        .enumerate()
        .filter(|(index, _session)| !removed.contains(index))
        .map(|(_index, session)| session)
        .collect();
    recompute_summary(&mut snapshot);
    snapshot
}

fn process_indices(sessions: &[SessionSnapshot]) -> Vec<usize> {
    sessions
        .iter()
        .enumerate()
        .filter(|(_index, session)| {
            is_process_only(session) && session.state != SessionState::Exited
        })
        .map(|(index, _session)| index)
        .collect()
}

fn is_process_only(session: &SessionSnapshot) -> bool {
    session.session_id.starts_with("process-")
}

fn merge_process_into_native(
    sessions: &mut [SessionSnapshot],
    process_index: usize,
    native_index: usize,
) {
    if process_index == native_index {
        return;
    }
    let process = sessions[process_index].clone();
    let native = &mut sessions[native_index];
    native.pid = native.pid.or(process.pid);
    if native.cwd.is_none() {
        native.cwd = process.cwd;
    }
    if process.started_at < native.started_at {
        native.started_at = process.started_at;
    }
    if process.last_seen_at > native.last_seen_at {
        native.last_seen_at = process.last_seen_at;
    }
}

fn recompute_summary(snapshot: &mut AppSnapshot) {
    snapshot.total_tps = snapshot
        .sessions
        .iter()
        .filter(|session| session.rate_unit == RateUnit::TokensPerSecond)
        .filter_map(|session| session.current_tps.value)
        .sum();
    snapshot.total_chars_per_second = snapshot
        .sessions
        .iter()
        .filter(|session| session.rate_unit == RateUnit::CharactersPerSecond)
        .filter_map(|session| session.current_tps.value)
        .sum();
    snapshot.active_sessions = snapshot
        .sessions
        .iter()
        .filter(|session| {
            matches!(
                session.state,
                SessionState::Queue
                    | SessionState::Stream
                    | SessionState::Tool
                    | SessionState::Input
                    | SessionState::Stall
                    | SessionState::Retry
            )
        })
        .count();
    snapshot.generating_sessions = snapshot
        .sessions
        .iter()
        .filter(|session| session.state == SessionState::Stream)
        .count();
    snapshot.stalled_sessions = snapshot
        .sessions
        .iter()
        .filter(|session| session.state == SessionState::Stall)
        .count();
    snapshot.error_sessions = snapshot
        .sessions
        .iter()
        .filter(|session| session.state == SessionState::Error)
        .count();
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::model::{Confidence, MetricValue};

    fn session(
        tool: ToolId,
        session_id: &str,
        pid: Option<u32>,
        state: SessionState,
    ) -> SessionSnapshot {
        let at = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
        SessionSnapshot {
            tool,
            session_id: session_id.to_owned(),
            state,
            turn_id: None,
            model: None,
            provider: None,
            cwd: None,
            pid,
            started_at: at,
            last_seen_at: at,
            current_tps: MetricValue::new(10.0, Confidence::Derived),
            turn_average_tps: MetricValue::unknown(),
            rate_unit: RateUnit::TokensPerSecond,
            ttft_ms: MetricValue::unknown(),
            e2e_ms: MetricValue::unknown(),
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            reasoning_tokens: 0,
            context_window: None,
            tool_wait_ms: 0,
            stall_ms: 0,
        }
    }

    fn snapshot(sessions: Vec<SessionSnapshot>) -> AppSnapshot {
        let generated_at = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
        AppSnapshot {
            generated_at,
            sessions,
            total_tps: 0.0,
            total_chars_per_second: 0.0,
            active_sessions: 0,
            generating_sessions: 0,
            stalled_sessions: 0,
            error_sessions: 0,
        }
    }

    #[test]
    fn exact_pid_match_merges_process_metadata_into_native_session() {
        let result = correlate_process_sessions(snapshot(vec![
            session(ToolId::Codex, "process-123", Some(123), SessionState::Unknown),
            session(ToolId::Codex, "native", Some(123), SessionState::Stream),
        ]));

        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].session_id, "native");
        assert_eq!(result.sessions[0].pid, Some(123));
        assert_eq!(result.sessions[0].state, SessionState::Stream);
        assert_eq!(result.total_tps, 10.0);
    }

    #[test]
    fn single_process_and_single_native_session_merge_without_pid_hint() {
        let result = correlate_process_sessions(snapshot(vec![
            session(ToolId::GrokBuild, "process-456", Some(456), SessionState::Unknown),
            session(ToolId::GrokBuild, "native", None, SessionState::Idle),
        ]));

        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].session_id, "native");
        assert_eq!(result.sessions[0].pid, Some(456));
    }

    #[test]
    fn ambiguous_processes_and_sessions_remain_separate() {
        let result = correlate_process_sessions(snapshot(vec![
            session(ToolId::Claude, "process-1", Some(1), SessionState::Unknown),
            session(ToolId::Claude, "process-2", Some(2), SessionState::Unknown),
            session(ToolId::Claude, "native-a", None, SessionState::Idle),
            session(ToolId::Claude, "native-b", None, SessionState::Idle),
        ]));

        assert_eq!(result.sessions.len(), 4);
    }

    #[test]
    fn exited_process_row_is_not_folded_into_a_live_native_session() {
        let result = correlate_process_sessions(snapshot(vec![
            session(ToolId::Pi, "process-9", Some(9), SessionState::Exited),
            session(ToolId::Pi, "native", None, SessionState::Idle),
        ]));

        assert_eq!(result.sessions.len(), 2);
    }

    #[test]
    fn live_process_is_not_folded_into_an_exited_native_session() {
        let result = correlate_process_sessions(snapshot(vec![
            session(ToolId::GrokBuild, "process-10", Some(10), SessionState::New),
            session(ToolId::GrokBuild, "native", Some(10), SessionState::Exited),
        ]));

        assert_eq!(result.sessions.len(), 2);
    }

    #[test]
    fn conflicting_pid_prevents_single_pair_fallback_merge() {
        let result = correlate_process_sessions(snapshot(vec![
            session(ToolId::Codex, "process-12", Some(12), SessionState::Unknown),
            session(ToolId::Codex, "native", Some(99), SessionState::Idle),
        ]));

        assert_eq!(result.sessions.len(), 2);
    }
}
