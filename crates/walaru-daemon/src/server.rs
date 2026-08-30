use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::net::{TcpListener, TcpStream};

#[cfg(unix)]
type LocalListener = UnixListener;
#[cfg(unix)]
type LocalStream = UnixStream;
#[cfg(windows)]
type LocalListener = TcpListener;
#[cfg(windows)]
type LocalStream = TcpStream;

use prost::Message;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use walaru_core::protocol::{
    Diagnostic, Envelope, NextAction, Page, RpcRequest, RpcResponse, SCHEMA_VERSION, Status,
};
use walaru_core::replay::{
    JvmReplayBackend, ReplayBackend, ReplayError, ReverseRequest, SourceTarget, StepKind,
};
use walaru_core::store::{ImpactSelection, RetentionPolicy, Store, StoreError};
use walaru_core::workspace::{RevisionSnapshot, WorkspaceError, WorkspaceLayout};

use crate::verifier::{
    RecordingOptions, RuntimeArtifacts, VerificationMode, VerificationRequest, Verifier,
    VerifierError,
};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Daemon startup, transport, or state error.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Workspace discovery failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// Local event store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Socket or metadata operation failed.
    #[error("daemon I/O error: {0}")]
    Io(#[from] io::Error),
    /// Protobuf frame was malformed.
    #[error("invalid protobuf frame: {0}")]
    Protobuf(#[from] prost::DecodeError),
    /// Response serialization failed.
    #[error("daemon JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Another healthy daemon already owns the socket.
    #[error("a Walaru daemon is already running at {0}")]
    AlreadyRunning(PathBuf),
    /// A peer announced an unsafe frame length.
    #[error("frame size {0} exceeds the 16 MiB local protocol limit")]
    FrameTooLarge(usize),
    /// Gradle verification or recording failed.
    #[error(transparent)]
    Verifier(#[from] VerifierError),
}

/// Command handler scoped to one canonical worktree.
#[derive(Debug)]
pub struct Daemon {
    layout: WorkspaceLayout,
    store: Store,
    session_id: String,
    should_stop: AtomicBool,
}

impl Daemon {
    /// Opens the worktree state and SQLite store.
    pub fn open(workspace: impl AsRef<Path>) -> Result<Self, DaemonError> {
        let layout = WorkspaceLayout::new(workspace)?;
        layout.ensure_state_dir()?;
        let store = Store::open(&layout.database, RetentionPolicy::default())?;
        store.prune(chrono::Utc::now())?;
        Ok(Self {
            layout,
            store,
            session_id: session_id(),
            should_stop: AtomicBool::new(false),
        })
    }

    /// Handles one total request and always returns a versioned response.
    #[must_use]
    pub fn handle(&self, request: RpcRequest) -> RpcResponse {
        let response = self.dispatch(&request).unwrap_or_else(|error| {
            (
                3,
                self.envelope(
                    Status::Error,
                    None,
                    json!({}),
                    vec![diagnostic("WALARU_INTERNAL", "error", &error.to_string())],
                ),
            )
        });
        let envelope_json = serde_json::to_vec(&response.1).unwrap_or_else(|_| b"{}".to_vec());
        RpcResponse {
            schema_version: 1,
            request_id: request.request_id,
            exit_code: response.0,
            envelope_json,
        }
    }

    /// Indicates that a valid `stop` request has been acknowledged.
    #[must_use]
    pub fn should_stop(&self) -> bool {
        self.should_stop.load(Ordering::Acquire)
    }

    /// Exposes the worktree store to the verifier and in-process adapters.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    fn dispatch(&self, request: &RpcRequest) -> Result<(i32, Envelope), DaemonError> {
        if request.schema_version != 1 {
            return Ok((
                2,
                self.envelope(
                    Status::Error,
                    None,
                    json!({"supportedSchemaVersion": 1}),
                    vec![diagnostic(
                        "WALARU_SCHEMA_VERSION",
                        "error",
                        &format!(
                            "wire schema {} is unsupported; expected 1",
                            request.schema_version
                        ),
                    )],
                ),
            ));
        }
        let request_root = Path::new(&request.workspace_root).canonicalize()?;
        if request_root != self.layout.root {
            return Ok((
                2,
                self.envelope(
                    Status::Error,
                    None,
                    json!({}),
                    vec![diagnostic(
                        "WALARU_WORKSPACE_MISMATCH",
                        "error",
                        "request workspace does not match the daemon worktree",
                    )],
                ),
            ));
        }

        let payload: RequestPayload = serde_json::from_slice(&request.payload_json)?;
        let query = payload.query.bounded();
        if request.command != "replay"
            && let Some(at) = query.at.as_deref()
        {
            let current = RevisionSnapshot::capture(&self.layout.root)?;
            if at != current.revision.as_str() {
                return Ok((
                    4,
                    self.envelope(
                        if at.starts_with("rev-") {
                            Status::Stale
                        } else {
                            Status::Unsupported
                        },
                        None,
                        json!({"requestedAt": at, "currentRevision": current.revision}),
                        vec![diagnostic(
                            "WALARU_QUERY_REVISION",
                            "error",
                            "historical query snapshots are unavailable; query the current revision",
                        )],
                    ),
                ));
            }
        }
        let requested_fields = query.fields.clone();
        let (mut exit_code, mut envelope) = match request.command.as_str() {
            "status" => Ok((
                0,
                self.envelope(
                    Status::Ok,
                    None,
                    json!({
                        "running": true,
                        "pid": std::process::id(),
                        "version": env!("CARGO_PKG_VERSION"),
                        "stateDirectory": self.layout.state_dir,
                        "database": self.store.path(),
                        "socket": self.layout.socket,
                    }),
                    Vec::new(),
                ),
            )),
            "stop" => {
                self.should_stop.store(true, Ordering::Release);
                Ok((
                    0,
                    self.envelope(Status::Ok, None, json!({"stopping": true}), Vec::new()),
                ))
            }
            "doctor" => Ok(self.doctor()),
            "verify" => self.verify(&payload),
            "tests" => {
                let result = self.store.tests(query.cursor.as_deref(), query.limit)?;
                let returned = result.items.len();
                let mut envelope =
                    self.envelope(Status::Ok, None, json!({"tests": result.items}), Vec::new());
                envelope.page = Some(Page {
                    cursor: query.cursor,
                    next_cursor: result.next_cursor,
                    limit: query.limit,
                    returned,
                });
                Ok((0, envelope))
            }
            "failure" => {
                let id = command_string(&payload.command, "id");
                let failure = self.store.failure(id)?;
                let mut envelope =
                    self.envelope(Status::Ok, None, json!({"failure": failure}), Vec::new());
                if let Some(failure) = failure {
                    envelope.next_actions.push(NextAction {
                        title: "Inspect test trace".into(),
                        argv: vec![
                            "walaru".into(),
                            "trace".into(),
                            failure.test_id,
                            "--format".into(),
                            "json".into(),
                        ],
                    });
                    if let Some(event_id) = failure.event_id {
                        envelope.next_actions.push(NextAction {
                            title: "Inspect failure values".into(),
                            argv: vec![
                                "walaru".into(),
                                "values".into(),
                                event_id,
                                "--format".into(),
                                "json".into(),
                            ],
                        });
                    }
                }
                Ok((0, envelope))
            }
            "impact" => {
                let subject = command_string(&payload.command, "subject");
                let selection = self.store.select_impact(subject, ":")?;
                let data = match selection {
                    ImpactSelection::Exact(tests) => {
                        json!({"subject": subject, "selection": "exact", "tests": tests})
                    }
                    ImpactSelection::ModuleAll { tests, reason } => json!({
                        "subject": subject,
                        "selection": "moduleAll",
                        "tests": tests,
                        "reason": reason,
                    }),
                };
                Ok((0, self.envelope(Status::Ok, None, data, Vec::new())))
            }
            "coverage" => {
                let subject = command_string(&payload.command, "subject");
                let result = self
                    .store
                    .coverage(subject, query.cursor.as_deref(), query.limit)?;
                let returned = result.items.len();
                let mut envelope = self.envelope(
                    Status::Ok,
                    None,
                    json!({"coverage": result.items}),
                    Vec::new(),
                );
                envelope.page = Some(Page {
                    cursor: query.cursor,
                    next_cursor: result.next_cursor,
                    limit: query.limit,
                    returned,
                });
                Ok((0, envelope))
            }
            "trace" => {
                let subject = command_string(&payload.command, "subject");
                // A field mask can remove large values. Fetch the requested item page first so
                // byte accounting happens on the projected response below rather than on the
                // unprojected event payload stored in SQLite.
                let event_max_bytes = if requested_fields.is_empty() {
                    query.max_bytes
                } else {
                    usize::MAX
                };
                let result = self.store.trace(
                    subject,
                    query.cursor.as_deref(),
                    query.limit,
                    event_max_bytes,
                )?;
                let returned = result.items.len();
                let mut envelope = self.envelope(
                    Status::Ok,
                    None,
                    json!({"events": result.items}),
                    Vec::new(),
                );
                envelope.page = Some(Page {
                    cursor: query.cursor,
                    next_cursor: result.next_cursor,
                    limit: query.limit,
                    returned,
                });
                Ok((0, envelope))
            }
            "values" => {
                let event_id = command_string(&payload.command, "event");
                let event = self.store.event(event_id)?;
                let data = event.map_or_else(
                    || json!({"eventId": event_id, "values": null}),
                    |event| {
                        json!({
                            "eventId": event.id,
                            "location": event.location,
                            "values": event.values,
                            "stateHash": event.state_hash,
                        })
                    },
                );
                Ok((0, self.envelope(Status::Ok, None, data, Vec::new())))
            }
            "record" => self.record(&payload),
            "replay" => self.replay_at(&payload, &query),
            "reverse" => self.reverse(&payload),
            other => Ok((
                2,
                self.envelope(
                    Status::Error,
                    None,
                    json!({"command": other}),
                    vec![diagnostic(
                        "WALARU_UNKNOWN_COMMAND",
                        "error",
                        &format!("unknown daemon command `{other}`"),
                    )],
                ),
            )),
        }?;
        apply_field_mask(&mut envelope.data, &requested_fields);
        if enforce_response_limit(&mut envelope, query.max_bytes) {
            exit_code = 4;
        }
        Ok((exit_code, envelope))
    }

    fn verify(&self, payload: &RequestPayload) -> Result<(i32, Envelope), DaemonError> {
        let full = payload
            .command
            .get("full")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let since = payload
            .command
            .get("since")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let verifier = Verifier::new(&self.layout, &self.store, RuntimeArtifacts::discover()?);
        let request = VerificationRequest {
            mode: VerificationMode::Fast,
            selected_tests: Vec::new(),
            full,
            since,
            capture_file_io: false,
        };
        let mut outcome = verifier.verify(&request)?;
        let requeued = outcome.status == walaru_core::store::RunStatus::Stale;
        if requeued {
            outcome = verifier.verify(&request)?;
        }
        let status = match outcome.status {
            walaru_core::store::RunStatus::Passed => Status::Ok,
            walaru_core::store::RunStatus::Failed => Status::Failure,
            walaru_core::store::RunStatus::Stale => Status::Stale,
            walaru_core::store::RunStatus::Error | walaru_core::store::RunStatus::Running => {
                Status::Error
            }
        };
        let mut diagnostics = Vec::new();
        if outcome.status == walaru_core::store::RunStatus::Failed {
            diagnostics.push(diagnostic(
                "WALARU_VERIFICATION_FAILED",
                "error",
                "compilation or selected tests failed; query failure IDs and trace",
            ));
        } else if outcome.status == walaru_core::store::RunStatus::Stale {
            diagnostics.push(diagnostic(
                "WALARU_STALE_REVISION",
                "warning",
                "workspace changed during verification; the result is not a success",
            ));
        }
        let mut data = serde_json::to_value(&outcome)?;
        if let Some(data) = data.as_object_mut() {
            data.insert("requeued".into(), json!(requeued));
        }
        let mut envelope = self.envelope(status, Some(outcome.run_id.clone()), data, diagnostics);
        envelope.revision = outcome.revision;
        envelope.capabilities = outcome.capabilities;
        Ok((outcome.exit_code, envelope))
    }

    fn record(&self, payload: &RequestPayload) -> Result<(i32, Envelope), DaemonError> {
        let test_id = command_string(&payload.command, "testId");
        let verifier = Verifier::new(&self.layout, &self.store, RuntimeArtifacts::discover()?);
        let recording = verifier.record_with_options(
            test_id,
            RecordingOptions {
                capture_file_io: payload
                    .command
                    .get("captureFileIo")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            },
        )?;
        let complete =
            recording.capabilities.completeness == walaru_core::protocol::Completeness::Complete;
        let mut envelope = self.envelope(
            if complete {
                Status::Ok
            } else {
                Status::Partial
            },
            None,
            json!({
                "recordingId": recording.id,
                "testId": recording.test_id,
                "revision": recording.revision,
                "events": recording.events.len(),
            }),
            if complete {
                Vec::new()
            } else {
                vec![diagnostic(
                    "WALARU_RECORDING_PARTIAL",
                    "warning",
                    "recording observed an unsupported boundary; inspect capabilities",
                )]
            },
        );
        envelope.capabilities = recording.capabilities;
        Ok((if complete { 0 } else { 4 }, envelope))
    }

    fn doctor(&self) -> (i32, Envelope) {
        let java = Command::new("java").arg("-version").output();
        let java_text = java
            .as_ref()
            .map(|output| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            })
            .unwrap_or_default();
        let java_major = parse_java_major(&java_text);
        let gradle_project = [
            "settings.gradle.kts",
            "settings.gradle",
            "build.gradle.kts",
            "build.gradle",
        ]
        .iter()
        .any(|file| self.layout.root.join(file).is_file());
        let gradle_wrapper = self.layout.root.join("gradlew").is_file()
            || self.layout.root.join("gradlew.bat").is_file();
        let maven_project = self.layout.root.join("pom.xml").is_file();
        let maven_wrapper =
            self.layout.root.join("mvnw").is_file() || self.layout.root.join("mvnw.cmd").is_file();
        let maven_path = Command::new(if cfg!(windows) { "mvn.cmd" } else { "mvn" })
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success());
        let (build_kind, build_project, build_wrapper, build_ready) = if gradle_project {
            ("gradle", true, gradle_wrapper, gradle_wrapper)
        } else if maven_project {
            ("maven", true, maven_wrapper, maven_wrapper || maven_path)
        } else {
            ("unknown", false, false, false)
        };
        let artifacts = RuntimeArtifacts::discover();
        let rr = cfg!(target_os = "linux")
            && Command::new("rr")
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success());
        let criu = cfg!(target_os = "linux")
            && Command::new("criu")
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success());
        let crac = Command::new("java")
            .args(["-XX:+PrintFlagsFinal", "-version"])
            .output()
            .is_ok_and(|output| {
                output.status.success()
                    && (String::from_utf8_lossy(&output.stdout).contains("CRaCCheckpointTo")
                        || String::from_utf8_lossy(&output.stderr).contains("CRaCCheckpointTo"))
            });
        let platform = matches!(std::env::consts::OS, "linux" | "macos" | "windows")
            && matches!(std::env::consts::ARCH, "x86_64" | "aarch64");
        let ready = java_major.is_some_and(|version| version >= 21)
            && build_ready
            && artifacts.is_ok()
            && platform;
        let mut diagnostics = Vec::new();
        if !platform {
            diagnostics.push(diagnostic(
                "WALARU_PLATFORM",
                "error",
                "Walaru supports Linux, macOS, and Windows on x86_64/aarch64",
            ));
        }
        if java_major.is_none_or(|version| version < 21) {
            diagnostics.push(diagnostic(
                "WALARU_JDK",
                "error",
                "JDK 21 or newer was not detected",
            ));
        }
        if gradle_project && !gradle_wrapper {
            diagnostics.push(diagnostic(
                "WALARU_GRADLE_WRAPPER",
                "warning",
                "target worktree has no Gradle Wrapper; PATH Gradle fallback is non-reproducible",
            ));
        }
        if maven_project && !maven_wrapper && !maven_path {
            diagnostics.push(diagnostic(
                "WALARU_MAVEN",
                "error",
                "target worktree has no Maven Wrapper and `mvn` was not detected on PATH",
            ));
        }
        if !build_project {
            diagnostics.push(diagnostic(
                "WALARU_BUILD_TOOL",
                "error",
                "neither a Gradle nor Maven JVM project was detected",
            ));
        }
        if let Err(error) = &artifacts {
            diagnostics.push(diagnostic(
                "WALARU_RUNTIME_ARTIFACTS",
                "error",
                &error.to_string(),
            ));
        }
        let data = json!({
            "ready": ready,
            "platform": {"os": std::env::consts::OS, "arch": std::env::consts::ARCH, "supported": platform},
            "java": {"major": java_major, "raw": java_text.lines().next().unwrap_or("")},
            "buildTool": {"kind": build_kind, "project": build_project, "wrapper": build_wrapper, "ready": build_ready},
            "gradleWrapper": {"present": gradle_wrapper, "path": self.layout.root.join(if cfg!(windows) { "gradlew.bat" } else { "gradlew" })},
            "maven": {"project": maven_project, "wrapper": maven_wrapper, "pathAvailable": maven_path},
            "runtimeArtifacts": {"present": artifacts.is_ok()},
            "linuxReplay": {
                "rr": rr,
                "criu": criu,
                "crac": crac,
                "checkpointAcceleration": criu && crac,
                "required": false
            },
        });
        (
            0,
            self.envelope(
                if ready { Status::Ok } else { Status::Partial },
                None,
                data,
                diagnostics,
            ),
        )
    }

    fn replay_at(
        &self,
        payload: &RequestPayload,
        query: &QueryOptions,
    ) -> Result<(i32, Envelope), DaemonError> {
        let recording_id = command_string(&payload.command, "recordingId");
        let Some(recording) = self.store.recording(recording_id)? else {
            return Ok((
                2,
                self.envelope(
                    Status::Error,
                    None,
                    json!({"recordingId": recording_id}),
                    vec![diagnostic(
                        "WALARU_RECORDING_NOT_FOUND",
                        "error",
                        &format!("recording `{recording_id}` was not found"),
                    )],
                ),
            ));
        };
        if recording.capabilities.completeness != walaru_core::protocol::Completeness::Complete {
            return Ok(self.capability_failure(&recording.capabilities));
        }
        let event_id = query.at.as_deref().unwrap_or("");
        let Some(event) = recording
            .events
            .iter()
            .find(|event| event.id == event_id)
            .cloned()
        else {
            return Ok((
                2,
                self.envelope(
                    Status::Error,
                    None,
                    json!({"recordingId": recording_id, "eventId": event_id}),
                    vec![diagnostic(
                        "WALARU_EVENT_NOT_FOUND",
                        "error",
                        &format!("event `{event_id}` is not present in `{recording_id}`"),
                    )],
                ),
            ));
        };
        let artifacts = match RuntimeArtifacts::discover() {
            Ok(artifacts) => artifacts,
            Err(error) => {
                return Ok(self.replay_verification_failure(
                    recording_id,
                    &recording.capabilities,
                    &error,
                ));
            }
        };
        let verifier = Verifier::new(&self.layout, &self.store, artifacts);
        match verifier.verify_replay_event(&recording, &event.id) {
            Ok(replay_run_id) => {
                let mut envelope = self.envelope(
                    Status::Ok,
                    Some(replay_run_id.clone()),
                    json!({
                        "recordingId": recording_id,
                        "replayRunId": replay_run_id,
                        "event": event,
                        "verified": true,
                    }),
                    Vec::new(),
                );
                envelope.capabilities = recording.capabilities;
                Ok((0, envelope))
            }
            Err(error) => {
                Ok(self.replay_verification_failure(recording_id, &recording.capabilities, &error))
            }
        }
    }

    fn reverse(&self, payload: &RequestPayload) -> Result<(i32, Envelope), DaemonError> {
        let recording_id = command_string(&payload.command, "recordingId");
        let Some(recording) = self.store.recording(recording_id)? else {
            return Ok((
                2,
                self.envelope(
                    Status::Error,
                    None,
                    json!({"recordingId": recording_id}),
                    vec![diagnostic(
                        "WALARU_RECORDING_NOT_FOUND",
                        "error",
                        &format!("recording `{recording_id}` was not found"),
                    )],
                ),
            ));
        };
        let step = payload
            .command
            .get("step")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| match value {
                "line" => Some(StepKind::Line),
                "call" => Some(StepKind::Call),
                "write" => Some(StepKind::Write),
                _ => None,
            });
        let until = payload
            .command
            .get("until")
            .and_then(serde_json::Value::as_str)
            .and_then(parse_source_target);
        let request = ReverseRequest {
            from_event_id: command_string(&payload.command, "from").into(),
            step,
            until,
            watch: payload
                .command
                .get("watch")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        };
        match JvmReplayBackend.reverse(&recording, &request) {
            Ok(outcome) => {
                let artifacts = match RuntimeArtifacts::discover() {
                    Ok(artifacts) => artifacts,
                    Err(error) => {
                        return Ok(self.replay_verification_failure(
                            recording_id,
                            &recording.capabilities,
                            &error,
                        ));
                    }
                };
                let verifier = Verifier::new(&self.layout, &self.store, artifacts);
                match verifier.verify_replay_event(&recording, &outcome.event.id) {
                    Ok(replay_run_id) => {
                        let mut envelope = self.envelope(
                            Status::Ok,
                            Some(replay_run_id.clone()),
                            json!({
                                "recordingId": recording_id,
                                "replayRunId": replay_run_id,
                                "backend": outcome.backend,
                                "event": outcome.event,
                                "verified": true,
                            }),
                            Vec::new(),
                        );
                        envelope.capabilities = recording.capabilities;
                        Ok((0, envelope))
                    }
                    Err(error) => Ok(self.replay_verification_failure(
                        recording_id,
                        &recording.capabilities,
                        &error,
                    )),
                }
            }
            Err(ReplayError::UnsupportedCapability { .. }) => {
                Ok(self.capability_failure(&recording.capabilities))
            }
            Err(error) => Ok((
                4,
                self.envelope(
                    Status::Unsupported,
                    None,
                    json!({"recordingId": recording_id}),
                    vec![diagnostic(
                        "WALARU_REPLAY_POSITION",
                        "error",
                        &error.to_string(),
                    )],
                ),
            )),
        }
    }

    fn capability_failure(
        &self,
        capabilities: &walaru_core::protocol::CapabilityManifest,
    ) -> (i32, Envelope) {
        let reason = capabilities
            .unavailable
            .iter()
            .next()
            .map_or("recording is not complete", |(_, reason)| reason.as_str());
        let mut envelope = self.envelope(
            Status::Unsupported,
            None,
            json!({}),
            vec![diagnostic("WALARU_REPLAY_CAPABILITY", "error", reason)],
        );
        envelope.capabilities = capabilities.clone();
        (4, envelope)
    }

    fn replay_verification_failure(
        &self,
        recording_id: &str,
        capabilities: &walaru_core::protocol::CapabilityManifest,
        error: &VerifierError,
    ) -> (i32, Envelope) {
        let internal = matches!(
            error,
            VerifierError::Store(_)
                | VerifierError::Io(_)
                | VerifierError::Json(_)
                | VerifierError::Timeout(_)
        );
        let mut envelope = self.envelope(
            if internal {
                Status::Error
            } else {
                Status::Unsupported
            },
            None,
            json!({"recordingId": recording_id, "verified": false}),
            vec![diagnostic(
                if internal {
                    "WALARU_REPLAY_WORKER"
                } else {
                    "WALARU_REPLAY_UNVERIFIED"
                },
                "error",
                &error.to_string(),
            )],
        );
        envelope.capabilities = capabilities.clone();
        (if internal { 3 } else { 4 }, envelope)
    }

    fn envelope(
        &self,
        status: Status,
        run_id: Option<String>,
        data: serde_json::Value,
        diagnostics: Vec<Diagnostic>,
    ) -> Envelope {
        let revision = RevisionSnapshot::capture(&self.layout.root).map_or_else(
            |_| format!("rev-{}", "0".repeat(64)),
            |snapshot| snapshot.revision.to_string(),
        );
        Envelope {
            schema_version: SCHEMA_VERSION.into(),
            workspace_id: self.layout.workspace_id.to_string(),
            revision,
            session_id: self.session_id.clone(),
            run_id,
            status,
            data,
            diagnostics,
            capabilities: JvmReplayBackend.capabilities(),
            next_actions: Vec::new(),
            page: None,
        }
    }
}

/// Blocking protobuf server for one worktree-local endpoint.
#[derive(Debug)]
pub struct DaemonServer;

impl DaemonServer {
    /// Runs until a `stop` request is served.
    pub fn serve(workspace: impl AsRef<Path>) -> Result<(), DaemonError> {
        let layout = WorkspaceLayout::new(workspace)?;
        layout.ensure_state_dir()?;
        let listener = bind_local_endpoint(&layout)?;
        let _guard = SocketGuard {
            socket: layout.socket.clone(),
            metadata: layout.daemon_metadata.clone(),
        };
        let daemon = Daemon::open(&layout.root)?;
        fs::write(
            &layout.daemon_metadata,
            serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
                "workspaceId": layout.workspace_id,
            }))?,
        )?;

        loop {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            stream.set_write_timeout(Some(Duration::from_secs(30)))?;
            let request = match read_message::<RpcRequest>(&mut stream) {
                Ok(request) => request,
                Err(DaemonError::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::BrokenPipe
                    ) =>
                {
                    continue;
                }
                // Malformed/untrusted local clients cannot terminate the worktree daemon.
                Err(DaemonError::Protobuf(_) | DaemonError::FrameTooLarge(_)) => continue,
                Err(error) => return Err(error),
            };
            let response = daemon.handle(request);
            if let Err(error) = write_message(&mut stream, &response)
                && !is_client_disconnect(&error)
            {
                return Err(error);
            }
            if daemon.should_stop() {
                break;
            }
        }
        Ok(())
    }
}

/// Sends one request to an already-running daemon.
pub fn send_request(socket: &Path, request: &RpcRequest) -> Result<RpcResponse, DaemonError> {
    let mut stream = connect_local_endpoint(socket)?;
    stream.set_read_timeout(Some(Duration::from_mins(6)))?;
    stream.set_write_timeout(Some(Duration::from_mins(1)))?;
    write_message(&mut stream, request)?;
    read_message(&mut stream)
}

/// Returns whether the worktree endpoint currently accepts local connections.
#[must_use]
pub fn daemon_is_running(endpoint: &Path) -> bool {
    connect_local_endpoint(endpoint).is_ok()
}

fn is_client_disconnect(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::Io(source)
            if matches!(
                source.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::UnexpectedEof
            )
    )
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct RequestPayload {
    query: QueryOptions,
    command: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct QueryOptions {
    fields: Vec<String>,
    limit: usize,
    cursor: Option<String>,
    max_bytes: usize,
    at: Option<String>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            fields: Vec::new(),
            limit: 100,
            cursor: None,
            max_bytes: 65_536,
            at: None,
        }
    }
}

impl QueryOptions {
    fn bounded(&self) -> Self {
        Self {
            fields: self.fields.clone(),
            limit: self.limit.clamp(1, 1_000),
            cursor: self.cursor.clone(),
            max_bytes: self.max_bytes.clamp(4_096, 1024 * 1024),
            at: self.at.clone(),
        }
    }
}

fn enforce_response_limit(envelope: &mut Envelope, max_bytes: usize) -> bool {
    let Ok(encoded) = serde_json::to_vec(envelope) else {
        return false;
    };
    if encoded.len() <= max_bytes {
        return false;
    }
    let diagnostic_codes = envelope
        .diagnostics
        .iter()
        .take(8)
        .map(|diagnostic| diagnostic.code.clone())
        .collect::<Vec<_>>();
    envelope.status = Status::Partial;
    envelope.data = json!({
        "truncated": true,
        "originalBytes": encoded.len(),
        "maxBytes": max_bytes,
        "diagnosticCodes": diagnostic_codes,
    });
    envelope.diagnostics = vec![diagnostic(
        "WALARU_RESPONSE_LIMIT",
        "warning",
        "the structured result exceeded --max-bytes; narrow --fields, --limit, or use pagination",
    )];
    envelope.next_actions.clear();
    envelope.page = None;
    true
}

fn apply_field_mask(data: &mut serde_json::Value, fields: &[String]) {
    if fields.is_empty() {
        return;
    }
    let paths = fields
        .iter()
        .map(|field| {
            field
                .split('.')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    *data = project_fields(data, &paths);
}

fn project_fields(source: &serde_json::Value, paths: &[Vec<String>]) -> serde_json::Value {
    match source {
        serde_json::Value::Object(object) => {
            let mut grouped: BTreeMap<&str, Vec<Vec<String>>> = BTreeMap::new();
            for path in paths {
                if let Some((head, tail)) = path.split_first() {
                    grouped.entry(head).or_default().push(tail.to_vec());
                }
            }
            let mut projected = serde_json::Map::new();
            for (key, tails) in grouped {
                let Some(value) = object.get(key) else {
                    continue;
                };
                let selected = if tails.iter().any(Vec::is_empty) {
                    value.clone()
                } else {
                    project_fields(value, &tails)
                };
                projected.insert(key.into(), selected);
            }
            serde_json::Value::Object(projected)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| project_fields(item, paths))
                .collect(),
        ),
        _ => source.clone(),
    }
}

fn write_message<M: Message>(stream: &mut LocalStream, message: &M) -> Result<(), DaemonError> {
    let payload = message.encode_to_vec();
    if payload.len() > MAX_FRAME_BYTES {
        return Err(DaemonError::FrameTooLarge(payload.len()));
    }
    let length =
        u32::try_from(payload.len()).map_err(|_| DaemonError::FrameTooLarge(payload.len()))?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()?;
    Ok(())
}

fn read_message<M: Message + Default>(stream: &mut LocalStream) -> Result<M, DaemonError> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(DaemonError::FrameTooLarge(length));
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload)?;
    Ok(M::decode(payload.as_slice())?)
}

#[cfg(unix)]
fn bind_local_endpoint(layout: &WorkspaceLayout) -> Result<LocalListener, DaemonError> {
    if layout.socket.exists() {
        if daemon_is_running(&layout.socket) {
            return Err(DaemonError::AlreadyRunning(layout.socket.clone()));
        }
        fs::remove_file(&layout.socket)?;
    }
    if layout.daemon_metadata.exists() {
        fs::remove_file(&layout.daemon_metadata)?;
    }
    let listener = UnixListener::bind(&layout.socket)?;
    if let Err(error) = fs::set_permissions(&layout.socket, fs::Permissions::from_mode(0o600)) {
        // Some sandboxed Unix filesystems reject chmod on socket inodes. The
        // containing worktree state directory is created with mode 0700.
        if error.kind() != io::ErrorKind::PermissionDenied && error.raw_os_error() != Some(1) {
            return Err(error.into());
        }
    }
    Ok(listener)
}

#[cfg(unix)]
fn connect_local_endpoint(endpoint: &Path) -> io::Result<LocalStream> {
    UnixStream::connect(endpoint)
}

#[cfg(windows)]
fn bind_local_endpoint(layout: &WorkspaceLayout) -> Result<LocalListener, DaemonError> {
    if layout.socket.exists() {
        if daemon_is_running(&layout.socket) {
            return Err(DaemonError::AlreadyRunning(layout.socket.clone()));
        }
        fs::remove_file(&layout.socket)?;
    }
    if layout.daemon_metadata.exists() {
        fs::remove_file(&layout.daemon_metadata)?;
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    fs::write(&layout.socket, format!("tcp://{address}\n"))?;
    Ok(listener)
}

#[cfg(windows)]
fn connect_local_endpoint(endpoint: &Path) -> io::Result<LocalStream> {
    let descriptor = fs::read_to_string(endpoint)?;
    let address = descriptor.trim().strip_prefix("tcp://").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid local endpoint descriptor",
        )
    })?;
    let parsed: std::net::SocketAddr = address.parse().map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "invalid local endpoint address")
    })?;
    if !parsed.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon endpoint is not loopback-only",
        ));
    }
    TcpStream::connect(parsed)
}

fn diagnostic(code: &str, severity: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        details: BTreeMap::new(),
    }
}

fn command_string<'a>(command: &'a serde_json::Value, name: &str) -> &'a str {
    command
        .get(name)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn parse_source_target(value: &str) -> Option<SourceTarget> {
    let (path, line) = value.rsplit_once(':')?;
    Some(SourceTarget {
        path: path.into(),
        line: line.parse().ok()?,
    })
}

fn parse_java_major(output: &str) -> Option<u32> {
    let quoted = output.split('"').nth(1)?;
    let first = quoted.split('.').next()?;
    if first == "1" {
        quoted.split('.').nth(1)?.parse().ok()
    } else {
        first.parse().ok()
    }
}

fn session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("session-{}-{nanos}", std::process::id())
}

struct SocketGuard {
    socket: PathBuf,
    metadata: PathBuf,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_file(&self.metadata);
    }
}
