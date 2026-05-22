//! tui: Interactive terminal dashboard for scrape-garden.

use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Frame, Terminal,
};
use shared::{DashboardMessage, DashboardState, JobStatus, LogEntry};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

// ─── TUI Application State ─────────────────────────────────────────────────

struct App {
    /// Latest dashboard state from the daemon.
    state: DashboardState,
    /// Selected job index in the table.
    selected_job: usize,
    /// Whether to quit.
    quit: bool,
    /// Scroll position for log stream.
    log_scroll: usize,
    /// Max log lines to display.
    max_log_lines: usize,
}

impl App {
    fn new() -> Self {
        Self {
            state: DashboardState::default(),
            selected_job: 0,
            quit: false,
            log_scroll: 0,
            max_log_lines: 20,
        }
    }

    fn update(&mut self, msg: DashboardMessage) {
        match msg {
            DashboardMessage::State(state) => {
                self.state = state;
                // Clamp selected index.
                if self.selected_job >= self.state.job_statuses.len() {
                    self.selected_job = self.state.job_statuses.len().saturating_sub(1);
                }
            }
            DashboardMessage::Log(entry) => {
                self.state.logs.push(entry);
                // Keep logs bounded.
                if self.state.logs.len() > 500 {
                    self.state.logs.drain(..self.state.logs.len() - 500);
                }
            }
            DashboardMessage::Result(result) => {
                // Update the job status in the state.
                let found = self
                    .state
                    .job_statuses
                    .iter_mut()
                    .find(|js| js.job_name == result.job_name);

                match found {
                    Some(js) => {
                        let status = result.status.clone();
                        js.status = status.clone();
                        js.last_run = Some(result.timestamp);
                        if status == JobStatus::Done {
                            js.run_count += 1;
                        } else if status == JobStatus::Error {
                            js.error_count += 1;
                        }
                    }
                    None => {
                        let status = result.status.clone();
                        let is_done = status == JobStatus::Done;
                        let is_error = status == JobStatus::Error;
                        self.state.job_statuses.push(shared::JobStatusEntry {
                            job_name: result.job_name,
                            status,
                            last_run: Some(result.timestamp),
                            run_count: if is_done { 1 } else { 0 },
                            error_count: if is_error { 1 } else { 0 },
                        });
                    }
                }

                // Update stats.
                self.state.stats.total_runs += 1;
                if result.status == JobStatus::Done {
                    self.state.stats.successful_runs += 1;
                } else if result.status == JobStatus::Error {
                    self.state.stats.failed_runs += 1;
                }
            }
        }
    }
}

// ─── Rendering ─────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, app: &App) {
    let size = frame.size();

    // Main layout: 3 columns — jobs table, stats, logs
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(20),
            Constraint::Percentage(40),
        ])
        .split(size);

    // ── Left: Job List ──────────────────────────────────────────────────
    render_jobs_table(frame, chunks[0], app);

    // ── Center: Stats ───────────────────────────────────────────────────
    render_stats(frame, chunks[1], &app.state);

    // ── Right: Log Stream ───────────────────────────────────────────────
    render_log_stream(frame, chunks[2], app);
}

fn render_jobs_table(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let header = vec!["Job".to_string(), "Status".to_string(), "Runs".to_string(), "Errors".to_string()];
    let rows: Vec<Row> = app
        .state
        .job_statuses
        .iter()
        .enumerate()
        .map(|(i, js)| {
            let status_color = match js.status {
                JobStatus::Done => Color::Green,
                JobStatus::Error => Color::Red,
                JobStatus::Running => Color::Yellow,
                JobStatus::Pending => Color::Gray,
            };
            let style = if i == app.selected_job {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(js.job_name.clone()),
                Cell::from(Span::raw(format!("{:?}", js.status)).style(status_color)),
                Cell::from(js.run_count.to_string()),
                Cell::from(js.error_count.to_string()),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        vec![
            Constraint::Percentage(35),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(Row::new(header).style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)))
    .block(Block::default().borders(Borders::ALL).title(" Jobs ").style(Style::default().fg(Color::Cyan)));

    frame.render_stateful_widget(table, area, &mut TableState::default());
}

fn render_stats(frame: &mut Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let stats = &state.stats;

    let lines = vec![
        Line::from(Span::raw("  scrape-garden").style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(format!("  Total Runs:     {}", stats.total_runs)),
        Line::from(format!("  Successful:     {}", stats.successful_runs)),
        Line::from(format!("  Failed:         {}", stats.failed_runs)),
        Line::from(format!("  Success Rate:   {:.1}%", if stats.total_runs > 0 {
            (stats.successful_runs as f64 / stats.total_runs as f64) * 100.0
        } else {
            0.0
        })),
        Line::from(""),
        Line::from(Span::raw("  Controls:").style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from("  ↑/↓  Navigate"),
        Line::from("  q      Quit"),
        Line::from("  r      Refresh"),
        Line::from("  c      Clear logs"),
    ];

    let text_block = ratatui::text::Text::from(lines);
    let paragraph = ratatui::widgets::Paragraph::new(text_block)
        .block(Block::default().borders(Borders::ALL).title(" Stats ").style(Style::default().fg(Color::Magenta)));

    frame.render_widget(paragraph, area);
}

fn render_log_stream(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let logs = &app.state.logs;
    let start = if logs.len() > app.max_log_lines {
        logs.len() - app.max_log_lines
    } else {
        0
    };
    let visible_logs: Vec<Line> = logs[start..]
        .iter()
        .map(|entry| {
            let level_color = match entry.level.as_str() {
                "ERROR" => Color::Red,
                "WARN" => Color::Yellow,
                "INFO" => Color::White,
                _ => Color::Gray,
            };
            let time = entry.timestamp.format("%H:%M:%S");
            Line::from(vec![
                Span::raw(format!("[{}] ", time)).style(Style::default().fg(Color::DarkGray)),
                Span::raw(format!("[{}] ", entry.level)).style(Style::default().fg(level_color)),
                Span::raw(&entry.message),
            ])
        })
        .collect();

    let paragraph = ratatui::widgets::Paragraph::new(ratatui::text::Text::from(visible_logs))
        .block(Block::default().borders(Borders::ALL).title(" Logs ").style(Style::default().fg(Color::Blue)));

    frame.render_widget(paragraph, area);
}

// ─── TUI Runner ────────────────────────────────────────────────────────────

/// Run the interactive TUI dashboard.
///
/// Spawns a background task that receives messages from the daemon channel
/// and updates the TUI state.
pub async fn run_tui(mut log_rx: mpsc::UnboundedReceiver<DashboardMessage>) -> Result<(), Box<dyn std::error::Error>> {
    // Set up terminal.
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app = Arc::new(std::sync::Mutex::new(App::new()));
    let running = Arc::new(AtomicBool::new(true));

    // Clone for signal handling.
    let running_clone = running.clone();
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::spawn(async move {
        let _ = ctrl_c.await;
        running_clone.store(false, Ordering::SeqCst);
    });

    // Background task: receive messages and update app state.
    let app_state = app.clone();
    let msg_task = tokio::spawn(async move {
        while let Some(msg) = log_rx.recv().await {
            let mut app = app_state.lock().unwrap();
            app.update(msg);
        }
    });

    // Main TUI loop.
    while running.load(Ordering::SeqCst) {
        terminal.draw(|frame| {
            let app = app.lock().unwrap();
            render(frame, &app);
        })?;

        // Process keyboard input (non-blocking).
        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                let mut app = app.lock().unwrap();
                match key.code {
                    crossterm::event::KeyCode::Char('q') => {
                        app.quit = true;
                        running.store(false, Ordering::SeqCst);
                    }
                    crossterm::event::KeyCode::Char('r') => {
                        // Refresh is handled by incoming messages.
                    }
                    crossterm::event::KeyCode::Char('c') => {
                        app.state.logs.clear();
                    }
                    crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                        if app.selected_job > 0 {
                            app.selected_job -= 1;
                        }
                    }
                    crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                        if app.selected_job + 1 < app.state.job_statuses.len() {
                            app.selected_job += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Cleanup.
    drop(msg_task);
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    terminal.show_cursor()?;

    Ok(())
}

// ─── Dashboard State Builder (for daemon to push state) ────────────────────

/// Build a DashboardState from current data — called by the daemon periodically.
pub fn build_dashboard_state(
    job_statuses: Vec<shared::JobStatusEntry>,
    logs: Vec<LogEntry>,
    stats: shared::Stats,
) -> DashboardState {
    DashboardState {
        job_statuses,
        logs,
        stats,
    }
}
