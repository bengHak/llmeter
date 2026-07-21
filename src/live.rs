mod correlation;
mod cursor;
mod source_catalog;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::aggregate::Aggregator;
use crate::discovery::{detect_processes, ProcessInfo};
use crate::model::{AppSnapshot, Confidence, EventKind, TelemetryEvent, ToolId};

use correlation::{correlate_process_sessions, retain_live_process_sessions};
use cursor::{JournalCursor, SourceCursor};
use source_catalog::scan_session_sources;

const DEFAULT_SOURCE_BOOTSTRAP_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_JOURNAL_BOOTSTRAP_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_EVENT_DEDUP_CAPACITY: usize = 131_072;

#[derive(Clone, Debug)]
pub struct LiveCollectorConfig {
    pub home: Option<PathBuf>,
    pub source_scan_interval: Duration,
    pub process_scan_interval: Duration,
    pub max_depth: usize,
    pub max_sources_per_tool: usize,
    pub max_sources_total: usize,
    pub source_bootstrap_bytes: u64,
    pub journal_bootstrap_bytes: u64,
    pub scan_processes: bool,
}

impl Default for LiveCollectorConfig {
    fn default() -> Self {
        Self {
            home: dirs::home_dir(),
            source_scan_interval: Duration::from_secs(2),
            process_scan_interval: Duration::from_secs(2),
            max_depth: 8,
            max_sources_per_tool: 24,
            max_sources_total: 128,
            source_bootstrap_bytes: DEFAULT_SOURCE_BOOTSTRAP_BYTES,
            journal_bootstrap_bytes: DEFAULT_JOURNAL_BOOTSTRAP_BYTES,
            scan_processes: true,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectorStats {
    pub source_count: usize,
    pub process_count: usize,
    pub bytes_read: u64,
    pub events_seen: u64,
    pub events_applied: u64,
    pub duplicate_events: u64,
    pub malformed_records: u64,
    pub source_errors: u64,
    pub source_resets: u64,
}

#[derive(Clone, Debug)]
struct KnownProcess {
    command: String,
    started_epoch_secs: i64,
    session_id: String,
}

#[derive(Debug)]
struct EventDeduper {
    capacity: usize,
    order: VecDeque<Uuid>,
    seen: HashSet<Uuid>,
}

impl EventDeduper {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity.min(4096)),
            seen: HashSet::with_capacity(capacity.min(4096)),
        }
    }

    fn insert(&mut self, id: Uuid) -> bool {
        if !self.seen.insert(id) {
            return false;
        }
        self.order.push_back(id);
        while self.order.len() > self.capacity {
            if let Some(expired) = self.order.pop_front() {
                self.seen.remove(&expired);
            }
        }
        true
    }
}

pub struct LiveCollector {
    config: LiveCollectorConfig,
    aggregator: Aggregator,
    journal: JournalCursor,
    sources: HashMap<PathBuf, SourceCursor>,
    processes: HashMap<(ToolId, u32), KnownProcess>,
    deduper: EventDeduper,
    next_source_scan: Option<Instant>,
    next_process_scan: Option<Instant>,
    stats: CollectorStats,
}

impl LiveCollector {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self::with_config(data_dir, LiveCollectorConfig::default())
    }

    pub fn with_config(
        data_dir: impl Into<PathBuf>,
        config: LiveCollectorConfig,
    ) -> Self {
        let data_dir = data_dir.into();
        Self {
            journal: JournalCursor::new(
                data_dir.join("events.jsonl"),
                config.journal_bootstrap_bytes,
            ),
            config,
            aggregator: Aggregator::default(),
            sources: HashMap::new(),
            processes: HashMap::new(),
            deduper: EventDeduper::new(DEFAULT_EVENT_DEDUP_CAPACITY),
            next_source_scan: None,
            next_process_scan: None,
            stats: CollectorStats::default(),
        }
    }

    pub fn stats(&self) -> &CollectorStats {
        &self.stats
    }

    pub async fn refresh(&mut self) -> Result<AppSnapshot> {
        self.refresh_at(Utc::now()).await
    }

    pub async fn refresh_at(&mut self, now: DateTime<Utc>) -> Result<AppSnapshot> {
        let mut new_events = Vec::new();

        if scan_due(self.next_source_scan) {
            self.refresh_source_catalog();
            schedule_next(
                &mut self.next_source_scan,
                self.config.source_scan_interval,
            );
        }
        if scan_due(self.next_process_scan) {
            self.refresh_processes(&now, &mut new_events);
            schedule_next(
                &mut self.next_process_scan,
                self.config.process_scan_interval,
            );
        }

        if journal_path_exists(&self.journal) {
            match self.journal.poll().await {
                Ok(poll) => self.record_poll(poll, &mut new_events),
                Err(_) => self.stats.source_errors = self.stats.source_errors.saturating_add(1),
            }
        }

        for cursor in self.sources.values_mut() {
            match cursor.poll(&now).await {
                Ok(poll) => record_cursor_poll(&mut self.stats, poll, &mut new_events),
                Err(_) => self.stats.source_errors = self.stats.source_errors.saturating_add(1),
            }
        }

        new_events.sort_by(|left, right| {
            left.occurred_at
                .cmp(&right.occurred_at)
                .then_with(|| left.observed_at.cmp(&right.observed_at))
        });
        self.stats.events_seen = self
            .stats
            .events_seen
            .saturating_add(new_events.len() as u64);
        for event in new_events {
            if self.deduper.insert(event.id) {
                self.aggregator.apply(event);
                self.stats.events_applied = self.stats.events_applied.saturating_add(1);
            } else {
                self.stats.duplicate_events = self.stats.duplicate_events.saturating_add(1);
            }
        }

        let snapshot = correlate_process_sessions(self.aggregator.snapshot(now));
        if self.config.scan_processes {
            let active = self.processes.keys().copied().collect();
            Ok(retain_live_process_sessions(snapshot, &active))
        } else {
            Ok(snapshot)
        }
    }

    fn refresh_source_catalog(&mut self) {
        let Some(home) = self.config.home.as_deref() else {
            self.sources.clear();
            self.stats.source_count = 0;
            return;
        };
        let discovered = scan_session_sources(
            home,
            self.config.max_depth,
            self.config.max_sources_per_tool,
            self.config.max_sources_total,
        );
        let active_paths = discovered
            .iter()
            .map(|source| &source.path)
            .cloned()
            .collect::<HashSet<_>>();
        self.sources
            .retain(|path, _cursor| active_paths.contains(path));
        let source_bootstrap_bytes = self.config.source_bootstrap_bytes;
        for source in discovered {
            let should_replace = self
                .sources
                .get(&source.path)
                .is_some_and(|cursor| cursor.tool() != source.tool);
            if should_replace {
                self.sources.remove(&source.path);
            }
            self.sources
                .entry(source.path.clone())
                .or_insert_with(|| {
                    SourceCursor::new(source.tool, source.path, source_bootstrap_bytes)
                });
        }
        self.stats.source_count = self.sources.len();
    }

    fn refresh_processes(&mut self, now: &DateTime<Utc>, output: &mut Vec<TelemetryEvent>) {
        if !self.config.scan_processes {
            self.stats.process_count = 0;
            return;
        }
        let detected = match detect_processes() {
            Ok(processes) => processes,
            Err(_) => {
                self.stats.source_errors = self.stats.source_errors.saturating_add(1);
                return;
            }
        };
        self.reconcile_processes(detected, now, output);
    }

    fn reconcile_processes(
        &mut self,
        detected: Vec<ProcessInfo>,
        now: &DateTime<Utc>,
        output: &mut Vec<TelemetryEvent>,
    ) {
        let mut current = HashSet::new();
        for process in detected {
            let Some(tool) = process.tool else {
                continue;
            };
            let key = (tool, process.pid);
            current.insert(key);
            match self.processes.get(&key).cloned() {
                Some(known)
                    if known.command == process.command
                        && same_process_start(known.started_epoch_secs, &process, now) =>
                {
                    output.push(process_heartbeat(tool, &known.session_id, now));
                }
                Some(known) => {
                    output.push(process_ended(tool, &known.session_id, now));
                    let started_epoch_secs = process_started_epoch_secs(&process, now);
                    let session_id = process_session_id(process.pid, started_epoch_secs);
                    output.push(process_discovered(
                        tool,
                        &session_id,
                        process.pid,
                        started_epoch_secs,
                        now,
                    ));
                    self.processes.insert(
                        key,
                        KnownProcess {
                            command: process.command,
                            started_epoch_secs,
                            session_id,
                        },
                    );
                }
                None => {
                    let started_epoch_secs = process_started_epoch_secs(&process, now);
                    let session_id = process_session_id(process.pid, started_epoch_secs);
                    output.push(process_discovered(
                        tool,
                        &session_id,
                        process.pid,
                        started_epoch_secs,
                        now,
                    ));
                    self.processes.insert(
                        key,
                        KnownProcess {
                            command: process.command,
                            started_epoch_secs,
                            session_id,
                        },
                    );
                }
            }
        }

        let exited = self
            .processes
            .keys()
            .copied()
            .filter(|key| !current.contains(key))
            .collect::<Vec<_>>();
        for key in exited {
            if let Some(known) = self.processes.remove(&key) {
                output.push(process_ended(key.0, &known.session_id, now));
            }
        }
        self.stats.process_count = self.processes.len();
    }

    fn record_poll(&mut self, poll: cursor::CursorPoll, output: &mut Vec<TelemetryEvent>) {
        record_cursor_poll(&mut self.stats, poll, output);
    }
}

fn record_cursor_poll(
    stats: &mut CollectorStats,
    poll: cursor::CursorPoll,
    output: &mut Vec<TelemetryEvent>,
) {
    stats.bytes_read = stats.bytes_read.saturating_add(poll.bytes_read);
    stats.malformed_records = stats
        .malformed_records
        .saturating_add(poll.malformed_records as u64);
    if poll.reset {
        stats.source_resets = stats.source_resets.saturating_add(1);
    }
    output.extend(poll.events);
}

fn scan_due(next: Option<Instant>) -> bool {
    next.is_none_or(|deadline| Instant::now() >= deadline)
}

fn schedule_next(next: &mut Option<Instant>, interval: Duration) {
    *next = Instant::now().checked_add(interval);
}

fn process_started_epoch_secs(process: &ProcessInfo, now: &DateTime<Utc>) -> i64 {
    let elapsed = i64::try_from(process.elapsed_secs.unwrap_or_default()).unwrap_or(i64::MAX);
    (*now - chrono::Duration::seconds(elapsed)).timestamp()
}

fn process_session_id(pid: u32, started_epoch_secs: i64) -> String {
    format!("process-{pid}-{started_epoch_secs}")
}

fn same_process_start(
    known_started_epoch_secs: i64,
    process: &ProcessInfo,
    now: &DateTime<Utc>,
) -> bool {
    process.elapsed_secs.is_none()
        || known_started_epoch_secs.abs_diff(process_started_epoch_secs(process, now)) <= 2
}

fn process_discovered(
    tool: ToolId,
    session_id: &str,
    pid: u32,
    started_epoch_secs: i64,
    now: &DateTime<Utc>,
) -> TelemetryEvent {
    let started_at = DateTime::from_timestamp(started_epoch_secs, 0).unwrap_or(*now);
    TelemetryEvent::new(
        tool,
        session_id,
        started_at,
        Confidence::Derived,
        EventKind::SessionDiscovered {
            pid: Some(pid),
            cwd: None,
            model: None,
        },
    )
    .with_observed_at(*now)
}

fn process_heartbeat(tool: ToolId, session_id: &str, now: &DateTime<Utc>) -> TelemetryEvent {
    TelemetryEvent::new(
        tool,
        session_id,
        *now,
        Confidence::Derived,
        EventKind::Heartbeat,
    )
}

fn process_ended(tool: ToolId, session_id: &str, now: &DateTime<Utc>) -> TelemetryEvent {
    TelemetryEvent::new(
        tool,
        session_id,
        *now,
        Confidence::Derived,
        EventKind::SessionEnded,
    )
}

fn journal_path_exists(journal: &JournalCursor) -> bool {
    journal.path().is_file()
}

static COLLECTORS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<LiveCollector>>>>> = OnceLock::new();

pub async fn load_cached_snapshot(data_dir: &Path) -> Result<AppSnapshot> {
    let key = collector_key(data_dir);
    let cache = COLLECTORS.get_or_init(|| Mutex::new(HashMap::new()));
    let collector = {
        let mut collectors = cache.lock().await;
        collectors
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(LiveCollector::new(key))))
            .clone()
    };
    let mut collector = collector.lock().await;
    collector.refresh().await
}

fn collector_key(data_dir: &Path) -> PathBuf {
    if data_dir.is_absolute() {
        return data_dir.to_path_buf();
    }
    std::env::current_dir()
        .map(|current| current.join(data_dir))
        .unwrap_or_else(|_| data_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn event_deduper_is_bounded_and_rejects_recent_duplicates() {
        let mut deduper = EventDeduper::new(2);
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let third = Uuid::new_v4();

        assert!(deduper.insert(first));
        assert!(!deduper.insert(first));
        assert!(deduper.insert(second));
        assert!(deduper.insert(third));
        assert!(deduper.insert(first));
    }

    #[test]
    fn process_session_id_is_stable_for_known_elapsed_time() {
        let now = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
        let process = ProcessInfo {
            pid: 42,
            parent_pid: Some(1),
            elapsed_secs: Some(60),
            command: "codex".to_owned(),
            tool: Some(ToolId::Codex),
        };
        assert_eq!(
            process_session_id(process.pid, process_started_epoch_secs(&process, &now)),
            "process-42-1784505540"
        );
    }

    #[test]
    fn process_reconciliation_treats_reused_pid_as_a_new_session() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = LiveCollectorConfig::default();
        config.scan_processes = true;
        let mut collector = LiveCollector::with_config(temp.path(), config);
        let first_at = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
        let second_at = first_at + chrono::Duration::seconds(20);
        let first = ProcessInfo {
            pid: 7,
            parent_pid: Some(1),
            elapsed_secs: Some(100),
            command: "grok".to_owned(),
            tool: Some(ToolId::GrokBuild),
        };
        let reused = ProcessInfo {
            pid: 7,
            parent_pid: Some(1),
            elapsed_secs: Some(1),
            command: "grok".to_owned(),
            tool: Some(ToolId::GrokBuild),
        };

        let mut initial = Vec::new();
        collector.reconcile_processes(vec![first], &first_at, &mut initial);
        let original_id = collector
            .processes
            .get(&(ToolId::GrokBuild, 7))
            .unwrap()
            .session_id
            .clone();

        let mut events = Vec::new();
        collector.reconcile_processes(vec![reused], &second_at, &mut events);
        let replacement_id = &collector
            .processes
            .get(&(ToolId::GrokBuild, 7))
            .unwrap()
            .session_id;

        assert_ne!(&original_id, replacement_id);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(&event.kind, EventKind::SessionEnded))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(&event.kind, EventKind::SessionDiscovered { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn process_reconciliation_emits_discovery_heartbeat_and_exit_once() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = LiveCollectorConfig::default();
        config.scan_processes = true;
        let mut collector = LiveCollector::with_config(temp.path(), config);
        let now = Utc.timestamp_millis_opt(1_784_505_600_000).unwrap();
        let process = ProcessInfo {
            pid: 7,
            parent_pid: Some(1),
            elapsed_secs: Some(10),
            command: "grok".to_owned(),
            tool: Some(ToolId::GrokBuild),
        };

        let mut first = Vec::new();
        collector.reconcile_processes(vec![process.clone()], &now, &mut first);
        assert_eq!(
            first
                .iter()
                .filter(|event| matches!(&event.kind, EventKind::SessionDiscovered { .. }))
                .count(),
            1
        );
        let discovered = first
            .iter()
            .find(|event| matches!(&event.kind, EventKind::SessionDiscovered { .. }))
            .unwrap();
        assert_eq!(
            discovered.occurred_at,
            now - chrono::Duration::seconds(10)
        );
        assert_eq!(discovered.observed_at, now);

        let mut second = Vec::new();
        collector.reconcile_processes(vec![process], &now, &mut second);
        assert_eq!(
            second
                .iter()
                .filter(|event| matches!(&event.kind, EventKind::Heartbeat))
                .count(),
            1
        );

        let mut third = Vec::new();
        collector.reconcile_processes(Vec::new(), &now, &mut third);
        assert_eq!(
            third
                .iter()
                .filter(|event| matches!(&event.kind, EventKind::SessionEnded))
                .count(),
            1
        );
        assert!(collector.processes.is_empty());
    }
}
