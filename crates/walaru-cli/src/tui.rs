//! Interactive tests and trace dashboard.

use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind};
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell as TableCell, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::{Frame, Terminal};
use serde_json::{Value, json};
use walaru_core::protocol::{Envelope, RpcRequest};
use walaru_core::workspace::WorkspaceLayout;
use walaru_daemon::send_request;

use super::{Cli, TuiArgs, request_id};

const MIN_WIDTH: u16 = 48;
const MIN_HEIGHT: u16 = 12;
const SNAPSHOT_WIDTH: u16 = 100;
const SNAPSHOT_HEIGHT: u16 = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Screen {
    Tests,
    Trace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LongOperation {
    Verify,
    FullVerify,
    Record,
}

impl LongOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::FullVerify => "full verify",
            Self::Record => "record",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TestItem {
    id: String,
    display_name: String,
    module: String,
    status: String,
    failure_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct TraceItem {
    id: String,
    sequence: u64,
    kind: String,
    location: String,
    thread_id: u64,
    values: Value,
}

#[derive(Debug)]
struct App {
    screen: Screen,
    tests: Vec<TestItem>,
    test_index: usize,
    trace: Vec<TraceItem>,
    trace_index: usize,
    failure_detail: Option<String>,
    trace_subject: Option<String>,
    revision: String,
    operation: Option<LongOperation>,
    notice: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            screen: Screen::Tests,
            tests: Vec::new(),
            test_index: 0,
            trace: Vec::new(),
            trace_index: 0,
            failure_detail: None,
            trace_subject: None,
            revision: "-".into(),
            operation: None,
            notice: "Ready".into(),
        }
    }
}

impl App {
    fn selected_test(&self) -> Option<&TestItem> {
        self.tests.get(self.test_index)
    }

    fn selected_trace(&self) -> Option<&TraceItem> {
        self.trace.get(self.trace_index)
    }

    fn move_down(&mut self) {
        let (index, length) = match self.screen {
            Screen::Tests => (&mut self.test_index, self.tests.len()),
            Screen::Trace => (&mut self.trace_index, self.trace.len()),
        };
        if length > 0 {
            *index = (*index + 1).min(length - 1);
        }
    }

    fn move_up(&mut self) {
        let index = match self.screen {
            Screen::Tests => &mut self.test_index,
            Screen::Trace => &mut self.trace_index,
        };
        *index = index.saturating_sub(1);
    }

    fn apply_tests(&mut self, envelope: &Envelope) {
        self.revision.clone_from(&envelope.revision);
        let selected_id = self.selected_test().map(|test| test.id.clone());
        let mut tests = envelope
            .data
            .get("tests")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| TestItem {
                id: text(item, "id"),
                display_name: text(item, "displayName"),
                module: text(item, "module"),
                status: text(item, "lastStatus"),
                failure_id: item
                    .get("lastFailureId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
            .collect::<Vec<_>>();
        tests.sort_by(|left, right| {
            failure_rank(&left.status)
                .cmp(&failure_rank(&right.status))
                .then_with(|| left.module.cmp(&right.module))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.tests = tests;
        self.test_index = selected_id
            .as_deref()
            .and_then(|id| self.tests.iter().position(|test| test.id == id))
            .unwrap_or(0)
            .min(self.tests.len().saturating_sub(1));
    }

    fn apply_trace(&mut self, envelope: &Envelope, subject: &str) {
        self.trace_subject = Some(subject.into());
        self.trace = envelope
            .data
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| TraceItem {
                id: text(item, "id"),
                sequence: item
                    .get("sequence")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                kind: text(item, "kind"),
                location: source_location(item.get("location")),
                thread_id: item
                    .get("threadId")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                values: item.get("values").cloned().unwrap_or(Value::Null),
            })
            .collect();
        self.trace_index = 0;
        self.screen = Screen::Trace;
    }

    fn apply_failure(&mut self, envelope: &Envelope) {
        self.failure_detail = envelope
            .data
            .get("failure")
            .filter(|failure| !failure.is_null())
            .map(|failure| {
                let mut lines = vec![
                    format!("{}", text(failure, "exceptionType")),
                    text(failure, "message"),
                ];
                if let Some(frames) = failure.get("frames").and_then(Value::as_array) {
                    lines.extend(
                        frames
                            .iter()
                            .filter_map(Value::as_str)
                            .take(5)
                            .map(|frame| format!("  {frame}")),
                    );
                }
                lines.join("\n")
            });
    }

    fn handle_key(&mut self, key: KeyEvent) -> Intent {
        if key.kind != KeyEventKind::Press {
            return Intent::None;
        }
        match key.code {
            KeyCode::Char('q') => Intent::Quit,
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                Intent::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                Intent::None
            }
            KeyCode::Enter if self.screen == Screen::Tests => Intent::OpenSelected,
            KeyCode::Esc if self.screen == Screen::Trace => {
                self.screen = Screen::Tests;
                Intent::None
            }
            KeyCode::Esc => {
                self.failure_detail = None;
                Intent::None
            }
            KeyCode::Char('r') => Intent::Refresh,
            KeyCode::Char('v') => Intent::Start(LongOperation::Verify),
            KeyCode::Char('f') => Intent::Start(LongOperation::FullVerify),
            KeyCode::Char('c') => Intent::Start(LongOperation::Record),
            _ => Intent::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Intent {
    None,
    Quit,
    Refresh,
    OpenSelected,
    Start(LongOperation),
}

#[derive(Clone, Debug)]
struct QueryConfig {
    workspace_root: String,
    limit: usize,
    max_bytes: usize,
    at: Option<String>,
}

impl QueryConfig {
    fn from_cli(cli: &Cli, layout: &WorkspaceLayout) -> Self {
        Self {
            workspace_root: layout.root.to_string_lossy().into_owned(),
            limit: cli.limit,
            max_bytes: cli.max_bytes,
            at: cli.at.clone(),
        }
    }

    fn request(&self, command: &str, command_payload: &Value) -> RpcRequest {
        RpcRequest {
            schema_version: 1,
            request_id: request_id(),
            workspace_root: self.workspace_root.clone(),
            command: command.into(),
            payload_json: serde_json::to_vec(&json!({
                "query": {
                    "fields": [],
                    "limit": self.limit,
                    "cursor": null,
                    "maxBytes": self.max_bytes,
                    "at": self.at,
                },
                "command": command_payload,
            }))
            .expect("JSON values are serializable"),
        }
    }
}

#[derive(Debug)]
struct OperationResult {
    operation: LongOperation,
    exit_code: i32,
    envelope: Envelope,
    tests: Option<Envelope>,
}

type OperationMessage = Result<OperationResult, String>;

pub(crate) fn is_interactive_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub(crate) fn run(
    cli: &Cli,
    layout: &WorkspaceLayout,
    arguments: &TuiArgs,
) -> Result<i32, Box<dyn std::error::Error>> {
    let config = QueryConfig::from_cli(cli, layout);
    let mut app = App::default();
    refresh_tests(&mut app, layout, &config)?;
    if arguments.once {
        return render_once(&app);
    }

    let (sender, receiver) = mpsc::channel();
    let _restore = RestoreGuard::new(ratatui::restore);
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(
        &mut terminal,
        &mut app,
        layout,
        &config,
        arguments,
        &sender,
        &receiver,
    );
    result.map(|()| 0)
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    layout: &WorkspaceLayout,
    config: &QueryConfig,
    arguments: &TuiArgs,
    sender: &Sender<OperationMessage>,
    receiver: &Receiver<OperationMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let interval = Duration::from_millis(arguments.interval_ms.max(100));
    let mut refreshed_at = Instant::now();
    loop {
        receive_operation(app, receiver);
        terminal.draw(|frame| render_frame(frame, app))?;

        if refreshed_at.elapsed() >= interval && app.operation.is_none() {
            match refresh_tests(app, layout, config) {
                Ok(()) => app.notice = "Refreshed".into(),
                Err(error) => app.notice = format!("Refresh failed: {error}"),
            }
            refreshed_at = Instant::now();
        }

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let TerminalEvent::Key(key) = event::read()? else {
            continue;
        };
        match app.handle_key(key) {
            Intent::None => {}
            Intent::Quit => return Ok(()),
            Intent::Refresh => {
                match refresh_tests(app, layout, config) {
                    Ok(()) => app.notice = "Refreshed".into(),
                    Err(error) => app.notice = format!("Refresh failed: {error}"),
                }
                refreshed_at = Instant::now();
            }
            Intent::OpenSelected => open_selected(app, layout, config),
            Intent::Start(operation) => {
                start_operation(app, layout, config, operation, sender);
            }
        }
    }
}

fn refresh_tests(
    app: &mut App,
    layout: &WorkspaceLayout,
    config: &QueryConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, envelope) = rpc(layout, config, "tests", &json!({}))?;
    app.apply_tests(&envelope);
    Ok(())
}

fn open_selected(app: &mut App, layout: &WorkspaceLayout, config: &QueryConfig) {
    let Some(test) = app.selected_test().cloned() else {
        app.notice = "No test selected".into();
        return;
    };
    if let Some(failure_id) = &test.failure_id {
        match rpc(layout, config, "failure", &json!({"id": failure_id})) {
            Ok((_, envelope)) => app.apply_failure(&envelope),
            Err(error) => app.notice = format!("Failure query failed: {error}"),
        }
    } else {
        app.failure_detail = None;
    }
    match rpc(layout, config, "trace", &json!({"subject": test.id})) {
        Ok((_, envelope)) => {
            app.apply_trace(&envelope, &test.id);
            app.notice = format!("Trace: {}", test.id);
        }
        Err(error) => app.notice = format!("Trace query failed: {error}"),
    }
}

fn start_operation(
    app: &mut App,
    layout: &WorkspaceLayout,
    config: &QueryConfig,
    operation: LongOperation,
    sender: &Sender<OperationMessage>,
) {
    if let Some(running) = app.operation {
        app.notice = format!("{} already in progress", running.label());
        return;
    }
    let test_id = app.selected_test().map(|test| test.id.clone());
    if operation == LongOperation::Record && test_id.is_none() {
        app.notice = "Select a test before recording".into();
        return;
    }
    app.operation = Some(operation);
    app.notice = format!("{} in progress…", operation.label());
    let layout = layout.clone();
    let config = config.clone();
    let sender = sender.clone();
    thread::spawn(move || {
        let (command, payload) = match operation {
            LongOperation::Verify => ("verify", json!({"full": false, "since": null})),
            LongOperation::FullVerify => ("verify", json!({"full": true, "since": null})),
            LongOperation::Record => (
                "record",
                json!({"testId": test_id.expect("record requires a selected test"), "captureFileIo": false}),
            ),
        };
        let result = rpc(&layout, &config, command, &payload)
            .and_then(|(exit_code, envelope)| {
                let tests = rpc(&layout, &config, "tests", &json!({}))?.1;
                Ok(OperationResult {
                    operation,
                    exit_code,
                    envelope,
                    tests: Some(tests),
                })
            })
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
}

fn receive_operation(app: &mut App, receiver: &Receiver<OperationMessage>) {
    let Ok(message) = receiver.try_recv() else {
        return;
    };
    app.operation = None;
    match message {
        Ok(result) => {
            if let Some(tests) = &result.tests {
                app.apply_tests(tests);
            }
            let outcome = if result.exit_code == 0 {
                "completed"
            } else {
                "failed"
            };
            app.notice = format!(
                "{} {outcome} (exit {}, status {:?})",
                result.operation.label(),
                result.exit_code,
                result.envelope.status
            );
        }
        Err(error) => app.notice = format!("Operation failed: {error}"),
    }
}

fn rpc(
    layout: &WorkspaceLayout,
    config: &QueryConfig,
    command: &str,
    payload: &Value,
) -> Result<(i32, Envelope), Box<dyn std::error::Error>> {
    let response = send_request(&layout.socket, &config.request(command, payload))?;
    let envelope = serde_json::from_slice(&response.envelope_json)?;
    Ok((response.exit_code, envelope))
}

fn render_once(app: &App) -> Result<i32, Box<dyn std::error::Error>> {
    let backend = TestBackend::new(SNAPSHOT_WIDTH, SNAPSHOT_HEIGHT);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render_frame(frame, app))?;
    let buffer = terminal.backend().buffer();
    let mut output = io::stdout().lock();
    for y in 0..SNAPSHOT_HEIGHT {
        let mut line = String::new();
        for x in 0..SNAPSHOT_WIDTH {
            line.push_str(buffer[(x, y)].symbol());
        }
        writeln!(output, "{}", line.trim_end())?;
    }
    Ok(0)
}

fn render_frame(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let warning = Paragraph::new(format!(
            "Walaru\nTerminal too small: {}×{}\nNeed at least {MIN_WIDTH}×{MIN_HEIGHT}",
            area.width, area.height
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Walaru dashboard"),
        )
        .wrap(Wrap { trim: true });
        frame.render_widget(warning, area);
        return;
    }

    let [header, body, detail, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(7),
        Constraint::Length(2),
    ])
    .areas(area);
    render_header(frame, app, header);
    match app.screen {
        Screen::Tests => render_tests(frame, app, body),
        Screen::Trace => render_trace(frame, app, body),
    }
    render_detail(frame, app, detail);
    render_footer(frame, app, footer);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let failed = app
        .tests
        .iter()
        .filter(|test| test.status == "failed")
        .count();
    let operation = app
        .operation
        .map_or_else(|| "idle".to_owned(), |value| format!("{}…", value.label()));
    let title = Line::from(vec![
        Span::styled(
            "Walaru dashboard",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  daemon: running  revision {}  tests {}  failures {}  {operation}",
            shortened(&app.revision, 16),
            app.tests.len(),
            failed
        )),
    ]);
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_tests(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(["Status", "Module", "Test"]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let rows = app.tests.iter().map(|test| {
        let style = if test.status == "failed" {
            Style::default().fg(Color::Red)
        } else if test.status == "passed" {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };
        Row::new([
            TableCell::from(test.status.as_str()),
            TableCell::from(test.module.as_str()),
            TableCell::from(test.id.as_str()),
        ])
        .style(style)
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Fill(1),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Tests · Enter opens trace"),
    )
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    let mut state = TableState::default();
    state.select((!app.tests.is_empty()).then_some(app.test_index));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_trace(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(["Seq", "Kind", "Source", "Thread"]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let rows = app.trace.iter().map(|event| {
        Row::new([
            TableCell::from(event.sequence.to_string()),
            TableCell::from(event.kind.as_str()),
            TableCell::from(event.location.as_str()),
            TableCell::from(event.thread_id.to_string()),
        ])
    });
    let title = format!(
        "Trace · {} · Esc returns",
        app.trace_subject.as_deref().unwrap_or("-")
    );
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Fill(1),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ");
    let mut state = TableState::default();
    state.select((!app.trace.is_empty()).then_some(app.trace_index));
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let (title, contents) = match app.screen {
        Screen::Tests => {
            let selected = app.selected_test();
            let title = selected.map_or_else(
                || "Failure detail".into(),
                |test| format!("Failure detail · {}", test.display_name),
            );
            let contents = app.failure_detail.clone().unwrap_or_else(|| {
                selected.map_or_else(
                    || "No test selected.".into(),
                    |test| {
                        test.failure_id.as_ref().map_or_else(
                            || "No recorded failure. Press Enter to inspect its trace.".into(),
                            |id| format!("Failure {id}. Press Enter to load details and trace."),
                        )
                    },
                )
            });
            (title, contents)
        }
        Screen::Trace => {
            let title = app
                .selected_trace()
                .map_or_else(|| "Values".into(), |event| format!("Values · {}", event.id));
            let contents = app.selected_trace().map_or_else(
                || "No trace events.".into(),
                |event| {
                    serde_json::to_string_pretty(&event.values).unwrap_or_else(|_| "null".into())
                },
            );
            (title, contents)
        }
    };
    frame.render_widget(
        Paragraph::new(contents)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let text = format!(
        "↑/↓ j/k move · Enter open · Esc back · r refresh · v verify · f full · c record · q quit\n{}",
        app.notice
    );
    frame.render_widget(Paragraph::new(text), area);
}

fn failure_rank(status: &str) -> u8 {
    match status {
        "failed" => 0,
        "passed" => 1,
        _ => 2,
    }
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("-").into()
}

fn source_location(value: Option<&Value>) -> String {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return "-".into();
    };
    format!(
        "{}:{}",
        text(value, "path"),
        value
            .get("line")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    )
}

fn shortened(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.into();
    }
    format!(
        "{}…",
        value
            .chars()
            .take(width.saturating_sub(1))
            .collect::<String>()
    )
}

struct RestoreGuard<F: FnMut()> {
    restore: F,
}

impl<F: FnMut()> RestoreGuard<F> {
    fn new(restore: F) -> Self {
        Self { restore }
    }
}

impl<F: FnMut()> Drop for RestoreGuard<F> {
    fn drop(&mut self) {
        (self.restore)();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    use super::{App, Intent, LongOperation, RestoreGuard, Screen, render_frame};
    use walaru_core::protocol::{CapabilityManifest, Completeness, Envelope, Status};

    fn envelope(data: serde_json::Value) -> Envelope {
        Envelope {
            schema_version: "1".into(),
            workspace_id: "ws-test".into(),
            revision: "rev-1234567890".into(),
            session_id: "session-test".into(),
            run_id: None,
            status: Status::Ok,
            data,
            diagnostics: Vec::new(),
            capabilities: CapabilityManifest {
                backend: "none".into(),
                completeness: Completeness::Unsupported,
                supported: Vec::new(),
                unavailable: BTreeMap::default(),
            },
            next_actions: Vec::new(),
            page: None,
        }
    }

    #[test]
    fn tests_are_failure_first_and_navigation_is_bounded() {
        let mut app = App::default();
        app.apply_tests(&envelope(json!({"tests": [
            {"id":"z#passes","displayName":"passes","module":":z","lastStatus":"passed","lastFailureId":null},
            {"id":"a#fails","displayName":"fails","module":":a","lastStatus":"failed","lastFailureId":"failure-1"}
        ]})));
        assert_eq!(app.tests[0].id, "a#fails");
        assert_eq!(app.test_index, 0);
        app.move_up();
        assert_eq!(app.test_index, 0);
        app.move_down();
        app.move_down();
        assert_eq!(app.test_index, 1);
    }

    #[test]
    fn keys_switch_views_and_request_long_operations() {
        let mut app = App::default();
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE)),
            Intent::Start(LongOperation::Verify)
        );
        app.screen = Screen::Trace;
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Intent::None
        );
        assert_eq!(app.screen, Screen::Tests);
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Intent::Quit
        );
    }

    #[test]
    fn test_backend_renders_normal_and_tiny_terminals_without_panicking() {
        let mut app = App::default();
        app.apply_tests(&envelope(json!({"tests": [{
            "id":"demo.ExampleTest#fails","displayName":"fails","module":":app",
            "lastStatus":"failed","lastFailureId":"failure-1"
        }]})));
        for (width, height) in [(100, 24), (20, 5)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| render_frame(frame, &app)).unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>();
            assert!(rendered.contains("Walaru"));
        }
    }

    #[test]
    fn restore_guard_always_restores_on_scope_exit() {
        let restored = Rc::new(Cell::new(false));
        {
            let restored = Rc::clone(&restored);
            let _guard = RestoreGuard::new(move || restored.set(true));
        }
        assert!(restored.get());
    }
}
