use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table, TableState, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::model::{AppSnapshot, MetricValue, RateUnit, SessionSnapshot, SessionState};
use crate::runtime::{load_snapshot, snapshot_from_journal};

#[derive(Clone, Debug)]
enum SnapshotSource {
    Live(PathBuf),
    Journal(PathBuf),
}

#[derive(Clone, Copy, Debug)]
enum SortMode {
    State,
    CurrentTps,
    Ttft,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            Self::State => Self::CurrentTps,
            Self::CurrentTps => Self::Ttft,
            Self::Ttft => Self::State,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::CurrentTps => "tps",
            Self::Ttft => "ttft",
        }
    }
}

struct App {
    source: SnapshotSource,
    snapshot: AppSnapshot,
    selected: usize,
    paused: bool,
    sort: SortMode,
    throughput_history: Vec<u64>,
    last_refresh: Instant,
    last_error: Option<String>,
}

impl App {
    async fn new(source: SnapshotSource) -> Self {
        let snapshot = refresh_source(&source)
            .await
            .unwrap_or_else(|_| AppSnapshot::empty(chrono::Utc::now()));
        Self {
            source,
            snapshot,
            selected: 0,
            paused: false,
            sort: SortMode::State,
            throughput_history: Vec::with_capacity(120),
            last_refresh: Instant::now(),
            last_error: None,
        }
    }

    async fn refresh(&mut self) {
        if self.paused {
            return;
        }
        match refresh_source(&self.source).await {
            Ok(mut snapshot) => {
                sort_sessions(&mut snapshot.sessions, self.sort);
                self.throughput_history
                    .push(snapshot.total_tps.max(0.0).round() as u64);
                if self.throughput_history.len() > 120 {
                    self.throughput_history.remove(0);
                }
                self.snapshot = snapshot;
                self.last_error = None;
                self.selected = self
                    .selected
                    .min(self.snapshot.sessions.len().saturating_sub(1));
            }
            Err(error) => self.last_error = Some(error.to_string()),
        }
        self.last_refresh = Instant::now();
    }

    fn select_next(&mut self) {
        if !self.snapshot.sessions.is_empty() {
            self.selected = (self.selected + 1).min(self.snapshot.sessions.len() - 1);
        }
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        sort_sessions(&mut self.snapshot.sessions, self.sort);
        self.selected = self
            .selected
            .min(self.snapshot.sessions.len().saturating_sub(1));
    }
}

pub async fn run_live(data_dir: &Path) -> Result<()> {
    run(SnapshotSource::Live(data_dir.to_path_buf())).await
}

pub async fn run_replay(journal: &Path) -> Result<()> {
    run(SnapshotSource::Journal(journal.to_path_buf())).await
}

struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

async fn run(source: SnapshotSource) -> Result<()> {
    let mut terminal = ratatui::init();
    let _restore_guard = TerminalRestoreGuard;
    run_loop(&mut terminal, source).await
}

async fn run_loop(terminal: &mut ratatui::DefaultTerminal, source: SnapshotSource) -> Result<()> {
    let mut app = App::new(source).await;
    app.refresh().await;

    loop {
        terminal.draw(|frame| render(frame, &app))?;

        if app.last_refresh.elapsed() >= Duration::from_secs(1) {
            app.refresh().await;
        }

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('j') | KeyCode::Down => app.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => app.select_previous(),
                    KeyCode::Char('p') => app.paused = !app.paused,
                    KeyCode::Char('s') => app.cycle_sort(),
                    KeyCode::Char('r') => app.refresh().await,
                    _ => {}
                },
                _ => {}
            }
        }
    }
    Ok(())
}

fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 60 || area.height < 16 {
        frame.render_widget(
            Paragraph::new("llmeter requires at least a 60x16 terminal")
                .block(Block::default().borders(Borders::ALL).title("llmeter"))
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let graph_height = if area.height >= 28 { 5 } else { 0 };
    let detail_height = if area.height >= 24 { 7 } else { 4 };
    let constraints = [
        Constraint::Length(3),
        Constraint::Length(graph_height),
        Constraint::Min(7),
        Constraint::Length(detail_height),
        Constraint::Length(1),
    ];
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_summary(frame, chunks[0], app);
    if graph_height > 0 {
        render_graph(frame, chunks[1], app);
    }
    render_sessions(frame, chunks[2], app);
    render_detail(frame, chunks[3], app);
    render_footer(frame, chunks[4], app);
}

fn render_summary(frame: &mut Frame, area: Rect, app: &App) {
    let summary = format!(
        "TOKEN {:>7.1} tok/s  sessions {}  active {}  generating {}  stalls {}  errors {}",
        app.snapshot.total_tps,
        app.snapshot.sessions.len(),
        app.snapshot.active_sessions,
        app.snapshot.generating_sessions,
        app.snapshot.stalled_sessions,
        app.snapshot.error_sessions,
    );
    let title = match &app.last_error {
        Some(error) => format!("llmeter · collector warning: {}", compact(error, 50)),
        None => "llmeter · live LLM session meter".to_owned(),
    };
    frame.render_widget(
        Paragraph::new(summary).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn render_graph(frame: &mut Frame, area: Rect, app: &App) {
    let max = app
        .throughput_history
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);
    frame.render_widget(
        Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("aggregate token throughput · 120s"),
            )
            .data(&app.throughput_history)
            .max(max),
        area,
    );
}

fn render_sessions(frame: &mut Frame, area: Rect, app: &App) {
    let compact_mode = area.width < 100;
    let rows = app.snapshot.sessions.iter().map(|session| {
        let model = session.model.as_deref().unwrap_or("-");
        let project = session
            .cwd
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("-");
        if compact_mode {
            Row::new(vec![
                Cell::from(session.state.label()),
                Cell::from(session.tool.as_str()),
                Cell::from(compact(model, 18)),
                Cell::from(format_rate(session.current_tps, session.rate_unit, 1)),
                Cell::from(format_metric(session.ttft_ms, 0)),
                Cell::from(session.output_tokens.to_string()),
            ])
        } else {
            Row::new(vec![
                Cell::from(session.state.label()),
                Cell::from(session.tool.as_str()),
                Cell::from(compact(model, 22)),
                Cell::from(compact(project, 20)),
                Cell::from(format_metric(session.ttft_ms, 0)),
                Cell::from(format_rate(session.current_tps, session.rate_unit, 1)),
                Cell::from(format_rate(session.turn_average_tps, session.rate_unit, 1)),
                Cell::from(format!("{:.1}s", session.stall_ms as f64 / 1_000.0)),
                Cell::from(session.output_tokens.to_string()),
            ])
        }
    });

    let (header, widths): (Row<'_>, Vec<Constraint>) = if compact_mode {
        (
            Row::new(["ST", "TOOL", "MODEL", "NOW(t/s)", "TTFT(ms)", "OUT(tok)"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
            vec![
                Constraint::Length(8),
                Constraint::Length(9),
                Constraint::Min(12),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(9),
            ],
        )
    } else {
        (
            Row::new([
                "ST",
                "TOOL",
                "MODEL",
                "PROJECT",
                "TTFT(ms)",
                "NOW(t/s)",
                "AVG(t/s)",
                "STALL(s)",
                "OUT(tok)",
            ])
            .style(Style::default().add_modifier(Modifier::BOLD)),
            vec![
                Constraint::Length(8),
                Constraint::Length(9),
                Constraint::Min(16),
                Constraint::Length(20),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(9),
                Constraint::Length(9),
            ],
        )
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("sessions"))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    let mut state = TableState::default();
    if !app.snapshot.sessions.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let text = app
        .snapshot
        .sessions
        .get(app.selected)
        .map(detail_text)
        .unwrap_or_else(|| {
            "No sessions discovered. Use `llmeter setup <tool>` or a wrapper command.".to_owned()
        });
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("selected session"),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let paused = if app.paused { "PAUSED" } else { "live" };
    frame.render_widget(
        Paragraph::new(format!(
            "j/k select  s sort:{}  p {}  r refresh  q quit",
            app.sort.label(),
            paused
        )),
        area,
    );
}

fn detail_text(session: &SessionSnapshot) -> String {
    format!(
        "{} / {}\nstate {}  turn {}  pid {}\nTTFT {}  E2E {}  now {}  avg {}\ninput {}  output {}  cached {}  reasoning {}  context {}\ntool wait {:.1}s  stall {:.1}s  cwd {}",
        session.tool,
        session.session_id,
        session.state.label(),
        session.turn_id.as_deref().unwrap_or("-"),
        session.pid.map_or_else(|| "-".to_owned(), |pid| pid.to_string()),
        format_metric(session.ttft_ms, 0),
        format_metric(session.e2e_ms, 0),
        format_rate(session.current_tps, session.rate_unit, 1),
        format_rate(session.turn_average_tps, session.rate_unit, 1),
        session.input_tokens,
        session.output_tokens,
        session.cached_input_tokens,
        session.reasoning_tokens,
        session.context_window.map_or_else(|| "-".to_owned(), |value| value.to_string()),
        session.tool_wait_ms as f64 / 1_000.0,
        session.stall_ms as f64 / 1_000.0,
        session.cwd.as_deref().unwrap_or("-"),
    )
}

fn format_metric(metric: MetricValue, precision: usize) -> String {
    let Some(value) = metric.value else {
        return "-".to_owned();
    };
    let formatted = match precision {
        0 => format!("{value:.0}"),
        1 => format!("{value:.1}"),
        _ => format!("{value}"),
    };
    format!("{}{}", metric.confidence.marker(), formatted)
}

fn format_rate(metric: MetricValue, unit: RateUnit, precision: usize) -> String {
    let formatted = format_metric(metric, precision);
    if formatted == "-" || unit == RateUnit::Unknown {
        formatted
    } else {
        format!("{formatted}{}", unit.compact_label())
    }
}

fn compact(value: &str, max_width: usize) -> String {
    if value.width() <= max_width {
        return value.to_owned();
    }
    if max_width <= 1 {
        return "…".to_owned();
    }
    let mut output = String::new();
    for character in value.chars() {
        let candidate_width = format!("{output}{character}…").width();
        if candidate_width > max_width {
            break;
        }
        output.push(character);
    }
    output.push('…');
    output
}

fn sort_sessions(sessions: &mut [SessionSnapshot], mode: SortMode) {
    match mode {
        SortMode::State => sessions.sort_by(|left, right| {
            session_state_rank(left.state)
                .cmp(&session_state_rank(right.state))
                .then_with(|| rate_unit_rank(left.rate_unit).cmp(&rate_unit_rank(right.rate_unit)))
                .then_with(|| compare_metric_desc(left.current_tps, right.current_tps))
                .then_with(|| left.tool.cmp(&right.tool))
                .then_with(|| left.session_id.cmp(&right.session_id))
        }),
        SortMode::CurrentTps => sessions.sort_by(|left, right| {
            rate_unit_rank(left.rate_unit)
                .cmp(&rate_unit_rank(right.rate_unit))
                .then_with(|| compare_metric_desc(left.current_tps, right.current_tps))
                .then_with(|| left.tool.cmp(&right.tool))
                .then_with(|| left.session_id.cmp(&right.session_id))
        }),
        SortMode::Ttft => sessions.sort_by(|left, right| {
            compare_optional_metric_asc(left.ttft_ms.value, right.ttft_ms.value)
                .then_with(|| left.tool.cmp(&right.tool))
                .then_with(|| left.session_id.cmp(&right.session_id))
        }),
    }
}

fn compare_metric_desc(left: MetricValue, right: MetricValue) -> std::cmp::Ordering {
    compare_optional_metric_desc(left.value, right.value)
}

fn compare_optional_metric_desc(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn compare_optional_metric_asc(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left
            .partial_cmp(&right)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn session_state_rank(state: SessionState) -> u8 {
    match state {
        SessionState::Error => 0,
        SessionState::Stall => 1,
        SessionState::Retry => 2,
        SessionState::Tool => 3,
        SessionState::Stream => 4,
        SessionState::Queue => 5,
        SessionState::Input => 6,
        SessionState::New => 7,
        SessionState::Idle => 8,
        SessionState::Exited => 9,
        SessionState::Unknown => 10,
    }
}

fn rate_unit_rank(unit: RateUnit) -> u8 {
    match unit {
        RateUnit::TokensPerSecond => 0,
        RateUnit::Unknown => 1,
    }
}

async fn refresh_source(source: &SnapshotSource) -> Result<AppSnapshot> {
    match source {
        SnapshotSource::Live(data_dir) => load_snapshot(data_dir).await,
        SnapshotSource::Journal(path) => snapshot_from_journal(path).await,
    }
}
