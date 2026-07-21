use std::collections::HashSet;

use uuid::Uuid;

use crate::model::{AppSnapshot, RateUnit, SessionSnapshot, SessionState, ToolId};

const START_MATCH_TOLERANCE_SECS: u64 = 60;

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

    let mut candidates = Vec::new();
    for process_index in process_indices(&snapshot.sessions) {
        if removed.contains(&process_index) {
            continue;
        }
        for (native_index, native) in snapshot.sessions.iter().enumerate() {
            if removed.contains(&native_index)
                || is_process_only(native)
                || native.state == SessionState::Exited
                || native.pid.is_some()
                || native.tool != snapshot.sessions[process_index].tool
            {
                continue;
            }
            let distance = session_start_epoch_secs(&snapshot.sessions[process_index])
                .abs_diff(session_start_epoch_secs(native));
            if distance <= START_MATCH_TOLERANCE_SECS {
                candidates.push((distance, process_index, native_index));
            }
        }
    }
    candidates.sort_unstable();

    let mut claimed_processes = HashSet::new();
    let mut claimed_natives = HashSet::new();
    for (_distance, process_index, native_index) in candidates {
        if claimed_processes.contains(&process_index) || claimed_natives.contains(&native_index) {
            continue;
        }
        claimed_processes.insert(process_index);
        claimed_natives.insert(native_index);
        merge_process_into_native(&mut snapshot.sessions, process_index, native_index);
        removed.insert(process_index);
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

pub fn retain_live_process_sessions(
    mut snapshot: AppSnapshot,
    active_processes: &HashSet<(ToolId, u32)>,
) -> AppSnapshot {
    snapshot.sessions.retain(|session| {
        session
            .pid
            .is_some_and(|pid| active_processes.contains(&(session.tool, pid)))
    });
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

fn session_start_epoch_secs(session: &SessionSnapshot) -> i64 {
    Uuid::parse_str(&session.session_id)
        .ok()
        .and_then(|id| id.get_timestamp())
        .and_then(|timestamp| i64::try_from(timestamp.to_unix().0).ok())
        .unwrap_or_else(|| session.started_at.timestamp())
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
        .fold(0.0_f64, |total, value| total + value);
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
            active_sessions: 0,
            generating_sessions: 0,
            stalled_sessions: 0,
            error_sessions: 0,
        }
    }

    #[test]
    fn empty_live_snapshot_uses_positive_zero_throughput() {
        let result = correlate_process_sessions(snapshot(Vec::new()));

        assert_eq!(result.total_tps, 0.0);
        assert!(result.total_tps.is_sign_positive());
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
    fn out_of_tolerance_processes_and_sessions_remain_separate() {
        let native_started = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
        let mut first_process =
            session(ToolId::Claude, "process-1", Some(1), SessionState::Unknown);
        first_process.started_at = native_started - chrono::Duration::minutes(2);
        let mut second_process =
            session(ToolId::Claude, "process-2", Some(2), SessionState::Unknown);
        second_process.started_at = native_started - chrono::Duration::minutes(3);
        let result = correlate_process_sessions(snapshot(vec![
            first_process,
            second_process,
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

    #[test]
    fn start_time_match_selects_current_native_and_live_filter_removes_history() {
        let process_started = Utc.timestamp_opt(1_784_639_940, 0).unwrap();
        let mut process = session(ToolId::GrokBuild, "process-7", Some(7), SessionState::New);
        process.started_at = process_started;
        let mut current = session(
            ToolId::GrokBuild,
            "019f84d4-737f-7b53-859b-7b519e34f571",
            None,
            SessionState::Stream,
        );
        current.started_at = process_started + chrono::Duration::minutes(5);
        let mut historical = session(
            ToolId::GrokBuild,
            "019f8493-10cf-7a71-b744-12a33cdfcf17",
            None,
            SessionState::Idle,
        );
        historical.last_seen_at = process_started + chrono::Duration::minutes(10);

        let correlated = correlate_process_sessions(snapshot(vec![process, historical, current]));
        let filtered =
            retain_live_process_sessions(correlated, &HashSet::from([(ToolId::GrokBuild, 7)]));

        assert_eq!(filtered.sessions.len(), 1);
        assert_eq!(
            filtered.sessions[0].session_id,
            "019f84d4-737f-7b53-859b-7b519e34f571",
        );
        assert_eq!(filtered.sessions[0].pid, Some(7));
    }

    #[test]
    fn start_time_match_claims_each_process_and_native_once() {
        let first_at = Utc.timestamp_opt(1_784_636_520, 0).unwrap();
        let second_at = Utc.timestamp_opt(1_784_640_410, 0).unwrap();
        let mut first_process = session(ToolId::Codex, "process-1", Some(1), SessionState::New);
        first_process.started_at = first_at;
        let mut second_process = session(ToolId::Codex, "process-2", Some(2), SessionState::New);
        second_process.started_at = second_at;
        let first_native = session(
            ToolId::Codex,
            "019f84a0-3971-79c1-b66e-37cb95a0de9c",
            None,
            SessionState::Stream,
        );
        let second_native = session(
            ToolId::Codex,
            "019f84db-942d-7920-936a-092152f36f01",
            None,
            SessionState::Stream,
        );

        let result = correlate_process_sessions(snapshot(vec![
            first_process,
            second_process,
            first_native,
            second_native,
        ]));
        let pids = result
            .sessions
            .iter()
            .filter_map(|session| session.pid)
            .collect::<HashSet<_>>();

        assert_eq!(result.sessions.len(), 2);
        assert_eq!(pids, HashSet::from([1, 2]));
    }

    #[test]
    fn live_filter_keeps_only_sessions_backed_by_a_running_process() {
        let active = HashSet::from([(ToolId::Codex, 12)]);
        let result = retain_live_process_sessions(
            snapshot(vec![
                session(ToolId::Claude, "stale-native", None, SessionState::Stall),
                session(ToolId::Codex, "live", Some(12), SessionState::Stream),
                session(ToolId::Codex, "exited", Some(99), SessionState::Exited),
            ]),
            &active,
        );

        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].session_id, "live");
        assert_eq!(result.active_sessions, 1);
        assert_eq!(result.generating_sessions, 1);
        assert_eq!(result.total_tps, 10.0);
    }
}
