//! `walaru` command-line client.

use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use walaru_core::protocol::{Envelope, RpcRequest};
use walaru_core::workspace::WorkspaceLayout;
use walaru_daemon::{DaemonServer, daemon_is_running, send_request};

mod human;
mod tui;

#[derive(Debug, Parser)]
#[command(
    name = "walaru",
    version,
    about = "Local-first JVM test intelligence and deterministic replay",
    disable_help_subcommand = true
)]
struct Cli {
    /// Target Gradle worktree (defaults to the current directory).
    #[arg(long, global = true, default_value = ".")]
    workspace: PathBuf,

    /// Output format; auto is human on a TTY and JSON otherwise.
    #[arg(long, global = true, value_enum, default_value = "auto")]
    format: OutputFormat,

    /// Comma-separated response fields for query data.
    #[arg(long, global = true, value_delimiter = ',')]
    fields: Vec<String>,

    /// Maximum query items.
    #[arg(long, global = true, default_value_t = 100)]
    limit: usize,

    /// Opaque pagination cursor.
    #[arg(long, global = true)]
    cursor: Option<String>,

    /// Maximum uncompressed response bytes.
    #[arg(long, global = true, default_value_t = 65_536)]
    max_bytes: usize,

    /// Revision/event snapshot for time-bound queries and replay.
    #[arg(long, global = true)]
    at: Option<String>,

    #[command(subcommand)]
    command: PublicCommand,
}

#[derive(Debug, Subcommand)]
enum PublicCommand {
    /// Show daemon, revision, and capability status.
    Status,
    /// Stream revision/status changes as NDJSON.
    Watch(WatchArgs),
    /// Show a live terminal dashboard backed by the structured query API.
    Tui(TuiArgs),
    /// Stop the worktree daemon.
    Stop,
    /// Diagnose JDK, Gradle, agent, and replay capabilities.
    Doctor,
    /// Compile and run conservatively selected tests.
    Verify(VerifyArgs),
    /// Verify, fully record failures, and explain them from local evidence.
    Explain(ExplainArgs),
    /// List discovered tests and last results.
    Tests,
    /// Explain one test failure.
    Failure { id: String },
    /// Select tests affected by a path or symbol.
    Impact { subject: String },
    /// Query test/source coverage.
    Coverage { subject: String },
    /// Query an ordered test or run trace.
    Trace { subject: String },
    /// Query safely captured values at an event.
    Values { event: String },
    /// Fully record one test in a fresh worker.
    Record {
        test_id: String,
        /// Explicitly persist supported bounded file inputs for deterministic replay.
        #[arg(long)]
        capture_file_io: bool,
    },
    /// Replay a recording at the global `--at` event.
    Replay { recording_id: String },
    /// Navigate to an earlier recorded state.
    Reverse(ReverseArgs),
    /// Internal detached daemon entry point.
    #[command(name = "__daemon", hide = true)]
    InternalDaemon,
}

#[derive(Debug, Args)]
struct WatchArgs {
    /// Polling period for the local status stream.
    #[arg(long, default_value_t = 500)]
    interval_ms: u64,
    /// Emit one item and return (intended for smoke tests).
    #[arg(long, hide = true)]
    once: bool,
}

#[derive(Debug, Args)]
struct TuiArgs {
    /// Dashboard refresh period.
    #[arg(long, default_value_t = 1_000)]
    interval_ms: u64,
    /// Render one dashboard frame and return (intended for automation and smoke tests).
    #[arg(long, hide = true)]
    once: bool,
}

#[derive(Debug, Args)]
struct VerifyArgs {
    /// Ignore impact history and execute all module tests.
    #[arg(long, conflicts_with = "since")]
    full: bool,
    /// Select changes since this VCS revision.
    #[arg(long, conflicts_with = "full")]
    since: Option<String>,
}

#[derive(Debug, Args)]
struct ExplainArgs {
    /// Ignore impact history and execute all module tests.
    #[arg(long, conflicts_with = "since")]
    full: bool,
    /// Select changes since this VCS revision.
    #[arg(long, conflicts_with = "full")]
    since: Option<String>,
    /// Maximum failed tests to fully record in this invocation.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(1..=20))]
    max_failures: u8,
}

#[derive(Debug, Args)]
struct ReverseArgs {
    /// Recording ID.
    recording_id: String,
    /// Current event (exclusive).
    #[arg(long)]
    from: String,
    /// Previous event boundary.
    #[arg(
        long,
        value_enum,
        conflicts_with = "until",
        required_unless_present = "until"
    )]
    step: Option<ReverseStep>,
    /// Reverse-continue target formatted as path:line.
    #[arg(long, conflicts_with = "step", required_unless_present = "step")]
    until: Option<String>,
    /// Stop at an exact `owner#field` or `array[index]` write watchpoint.
    #[arg(long, requires = "step", conflicts_with = "until")]
    watch: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ReverseStep {
    Line,
    Call,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    Auto,
    Human,
    Json,
    Ndjson,
}

fn main() {
    let cli = Cli::parse();
    if matches!(cli.command, PublicCommand::InternalDaemon) {
        match DaemonServer::serve(&cli.workspace) {
            Ok(()) => return,
            Err(error) => {
                eprintln!("walaru daemon: {error}");
                std::process::exit(3);
            }
        }
    }
    if matches!(cli.command, PublicCommand::Replay { .. }) && cli.at.is_none() {
        eprintln!("error: replay requires --at <event-id>");
        std::process::exit(2);
    }
    if let PublicCommand::Reverse(arguments) = &cli.command
        && arguments.watch.is_some()
        && arguments.step != Some(ReverseStep::Write)
    {
        eprintln!("error: --watch requires --step write");
        std::process::exit(2);
    }
    let exit_code = match run_client(&cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("walaru: {error}");
            3
        }
    };
    std::process::exit(exit_code);
}

fn run_client(cli: &Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let layout = WorkspaceLayout::new(&cli.workspace)?;
    if let PublicCommand::Tui(arguments) = &cli.command
        && !arguments.once
        && !tui::is_interactive_terminal()
    {
        eprintln!("error: interactive TUI requires a TTY; use `walaru tui --once` for automation");
        return Ok(2);
    }
    ensure_daemon(&layout)?;
    if let PublicCommand::Watch(arguments) = &cli.command {
        return watch(cli, &layout, arguments);
    }
    if let PublicCommand::Tui(arguments) = &cli.command {
        return tui::run(cli, &layout, arguments);
    }

    let (command, command_payload) = command_payload(&cli.command);
    let request = request(cli, &layout, command, &command_payload);
    let response = send_request(&layout.socket, &request)?;
    let envelope: Envelope = serde_json::from_slice(&response.envelope_json)?;
    render(&envelope, effective_format(cli.format, false), command)?;
    Ok(response.exit_code)
}

fn watch(
    cli: &Cli,
    layout: &WorkspaceLayout,
    arguments: &WatchArgs,
) -> Result<i32, Box<dyn std::error::Error>> {
    loop {
        let request = request(cli, layout, "status", &json!({"watch": true}));
        let response = send_request(&layout.socket, &request)?;
        let envelope: Envelope = serde_json::from_slice(&response.envelope_json)?;
        render(&envelope, effective_format(cli.format, true), "status")?;
        if arguments.once || response.exit_code != 0 {
            return Ok(response.exit_code);
        }
        thread::sleep(Duration::from_millis(arguments.interval_ms.max(25)));
    }
}

fn command_payload(command: &PublicCommand) -> (&'static str, Value) {
    match command {
        PublicCommand::Status => ("status", json!({})),
        PublicCommand::Stop => ("stop", json!({})),
        PublicCommand::Doctor => ("doctor", json!({})),
        PublicCommand::Verify(arguments) => (
            "verify",
            json!({"full": arguments.full, "since": arguments.since}),
        ),
        PublicCommand::Explain(arguments) => (
            "explain",
            json!({
                "full": arguments.full,
                "since": arguments.since,
                "maxFailures": arguments.max_failures,
            }),
        ),
        PublicCommand::Tests => ("tests", json!({})),
        PublicCommand::Failure { id } => ("failure", json!({"id": id})),
        PublicCommand::Impact { subject } => ("impact", json!({"subject": subject})),
        PublicCommand::Coverage { subject } => ("coverage", json!({"subject": subject})),
        PublicCommand::Trace { subject } => ("trace", json!({"subject": subject})),
        PublicCommand::Values { event } => ("values", json!({"event": event})),
        PublicCommand::Record {
            test_id,
            capture_file_io,
        } => (
            "record",
            json!({"testId": test_id, "captureFileIo": capture_file_io}),
        ),
        PublicCommand::Replay { recording_id } => ("replay", json!({"recordingId": recording_id})),
        PublicCommand::Reverse(arguments) => (
            "reverse",
            json!({
                "recordingId": arguments.recording_id,
                "from": arguments.from,
                "step": arguments.step.map(|step| match step {
                    ReverseStep::Line => "line",
                    ReverseStep::Call => "call",
                    ReverseStep::Write => "write",
                }),
                "until": arguments.until,
                "watch": arguments.watch,
            }),
        ),
        PublicCommand::Watch(_) | PublicCommand::Tui(_) | PublicCommand::InternalDaemon => {
            unreachable!()
        }
    }
}

fn request(
    cli: &Cli,
    layout: &WorkspaceLayout,
    command: &str,
    command_payload: &Value,
) -> RpcRequest {
    RpcRequest {
        schema_version: 1,
        request_id: request_id(),
        workspace_root: layout.root.to_string_lossy().into_owned(),
        command: command.into(),
        payload_json: serde_json::to_vec(&json!({
            "query": {
                "fields": cli.fields,
                "limit": cli.limit,
                "cursor": cli.cursor,
                "maxBytes": cli.max_bytes,
                "at": cli.at,
            },
            "command": command_payload,
        }))
        .expect("JSON values are serializable"),
    }
}

fn ensure_daemon(layout: &WorkspaceLayout) -> Result<(), Box<dyn std::error::Error>> {
    if daemon_ready(layout) {
        return Ok(());
    }
    layout.ensure_state_dir()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let lock_path = layout.state_dir.join("daemon.starting.lock");
    loop {
        if daemon_ready(layout) {
            return Ok(());
        }
        if let Some(_lock) = StartupLock::try_acquire(&lock_path)? {
            // Readiness can change between the optimistic check and acquiring the lock.
            // Rechecking here prevents a delayed client from starting a second daemon.
            if daemon_ready(layout) {
                return Ok(());
            }
            return start_daemon(layout);
        }
        if Instant::now() >= deadline {
            if daemon_ready(layout) {
                return Ok(());
            }
            return Err("timed out waiting for another client to start the daemon".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn daemon_ready(layout: &WorkspaceLayout) -> bool {
    layout.socket.exists() && layout.daemon_metadata.exists() && daemon_is_running(&layout.socket)
}

fn start_daemon(layout: &WorkspaceLayout) -> Result<(), Box<dyn std::error::Error>> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(layout.state_dir.join("daemon.log"))?;
    let mut child = ProcessCommand::new(std::env::current_exe()?)
        .arg("--workspace")
        .arg(&layout.root)
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut child_status = None;
    while Instant::now() < deadline {
        if daemon_ready(layout) {
            return Ok(());
        }
        if child_status.is_none() {
            child_status = child.try_wait()?;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if let Some(status) = child_status {
        return Err(format!("daemon exited during startup with {status}").into());
    }
    // Do not leave an unready child behind when the startup lock is released. In particular,
    // this keeps Windows job objects and their inherited handles from outliving a failed client.
    let _ = child.kill();
    let _ = child.wait();
    Err(format!("daemon did not create {}", layout.socket.display()).into())
}

struct StartupLock {
    file: File,
}

impl StartupLock {
    fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        let file = match OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if lock_is_contended(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Some(Self { file })),
            Err(error) if lock_is_contended(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn lock_is_contended(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        // LockFileEx contention can surface while opening or locking the shared file.
        // ERROR_SHARING_VIOLATION is 32 and ERROR_LOCK_VIOLATION is 33.
        matches!(error.raw_os_error(), Some(32 | 33))
    }
    #[cfg(not(windows))]
    false
}

impl Drop for StartupLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn effective_format(requested: OutputFormat, watch: bool) -> OutputFormat {
    match requested {
        OutputFormat::Auto if watch => OutputFormat::Ndjson,
        OutputFormat::Auto if io::stdout().is_terminal() => OutputFormat::Human,
        OutputFormat::Auto => OutputFormat::Json,
        explicit => explicit,
    }
}

fn render(
    envelope: &Envelope,
    format: OutputFormat,
    command: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    match format {
        OutputFormat::Human => {
            human::render(&mut output, command, envelope)?;
        }
        OutputFormat::Json | OutputFormat::Ndjson => {
            serde_json::to_writer(&mut output, envelope)?;
            writeln!(output).ok();
        }
        OutputFormat::Auto => unreachable!(),
    }
    Ok(())
}

fn request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("request-{}-{nanos}", std::process::id())
}
