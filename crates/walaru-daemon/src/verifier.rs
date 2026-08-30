use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Utc;
#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use thiserror::Error;
use walaru_core::event::{EventId, EventIdentity};
use walaru_core::protocol::{CapabilityManifest, Completeness};
use walaru_core::replay::{
    Event, EventKind, LogicalFrame, RecordedInput, Recording, ReplayCheckpoint, ReplayError,
    SourceLocation, verify_replayed_prefix,
};
use walaru_core::store::{
    CoverageRecord, Dependency, FailureRecord, ImpactSelection, NewRun, RevisionManifest,
    RunStatus, Store, StoreError, TestRecord,
};
use walaru_core::workspace::{
    RevisionSnapshot, WorkspaceError, WorkspaceLayout, source_surface_digest,
};

const MAX_EVENT_LINE_BYTES: usize = 1024 * 1024;
const MAX_EVENTS_PER_RUN: usize = 1_000_000;
const MAX_WORKER_LOG_BYTES: u64 = 8 * 1024 * 1024;

/// JVM artifacts injected into an unmodified Gradle worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeArtifacts {
    /// Fat optional-plugin/init-script adapter jar.
    pub adapter_jar: PathBuf,
    /// Fat Java ASM agent jar.
    pub agent_jar: PathBuf,
    /// Zero-config Gradle init script.
    pub init_script: PathBuf,
}

impl RuntimeArtifacts {
    /// Finds explicit environment overrides, a release layout, or this development checkout.
    pub fn discover() -> Result<Self, VerifierError> {
        let development_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("daemon crate is nested under the workspace");
        let executable = std::env::current_exe()?;
        let release_root = executable
            .parent()
            .and_then(Path::parent)
            .unwrap_or(development_root);
        let workspace_version = env!("CARGO_PKG_VERSION");
        let adapter_jar = artifact_path(
            "WALARU_ADAPTER_JAR",
            &[
                release_root.join("lib/walaru-gradle-adapter.jar"),
                development_root.join(format!(
                    "gradle-adapter/build/libs/gradle-adapter-{workspace_version}-all.jar"
                )),
            ],
        )?;
        let agent_jar = artifact_path(
            "WALARU_AGENT_JAR",
            &[
                release_root.join("lib/walaru-agent.jar"),
                development_root.join(format!(
                    "jvm-agent/build/libs/jvm-agent-{workspace_version}-all.jar"
                )),
            ],
        )?;
        let init_script = artifact_path(
            "WALARU_INIT_SCRIPT",
            &[
                release_root.join("share/walaru/walaru.init.gradle.kts"),
                development_root.join("gradle/walaru.init.gradle.kts"),
            ],
        )?;
        Ok(Self {
            adapter_jar,
            agent_jar,
            init_script,
        })
    }
}

/// Instrumentation profile for a worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationMode {
    /// Deduplicated coverage and dependency events.
    Fast,
    /// Ordered line/call/write/value events in a fresh worker.
    Full,
}

impl VerificationMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Full => "full",
        }
    }
}

/// One verification/recording request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationRequest {
    /// Instrumentation profile.
    pub mode: VerificationMode,
    /// Public tests selected by impact analysis.
    pub selected_tests: Vec<String>,
    /// Whether selection was explicitly widened to every test.
    pub full: bool,
    /// Optional VCS comparison reference.
    pub since: Option<String>,
    /// Explicitly capture supported bounded file inputs in the private replay tape.
    pub capture_file_io: bool,
}

impl VerificationRequest {
    /// Default conservative fast verification.
    #[must_use]
    pub fn fast() -> Self {
        Self {
            mode: VerificationMode::Fast,
            selected_tests: Vec::new(),
            full: false,
            since: None,
            capture_file_io: false,
        }
    }
}

/// Explicit recording choices that may persist otherwise-redacted external inputs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecordingOptions {
    /// Capture supported file reads up to the agent's fixed byte bound.
    pub capture_file_io: bool,
}

/// Persisted verification result returned to the daemon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationOutcome {
    /// Run ID.
    pub run_id: String,
    /// Revision seen before worker launch.
    pub revision: String,
    /// Stale-safe terminal state.
    pub status: RunStatus,
    /// Public CLI exit code.
    pub exit_code: i32,
    /// Discovered public test IDs.
    pub tests: Vec<String>,
    /// Structured failure IDs.
    pub failures: Vec<String>,
    /// Persisted event count.
    pub events: usize,
    /// Recording guarantee derived from observed boundaries.
    pub capabilities: CapabilityManifest,
    /// Worker log for local diagnostics.
    pub log_file: PathBuf,
    /// Whether Gradle execution was skipped using an exact successful revision cache.
    pub cached: bool,
    /// `cached`, `impact`, `explicit`, `full`, or conservative `moduleAll`.
    pub selection: String,
    /// Exact public test IDs passed to Gradle, empty when widened to the module.
    pub selected_tests: Vec<String>,
}

/// Gradle worker or ingestion failure.
#[derive(Debug, Error)]
pub enum VerifierError {
    /// Workspace capture/layout failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// Event store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Worker launch, log, or event read failed.
    #[error("verification I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// JVM event stream contained invalid JSON.
    #[error("invalid JVM event JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Required release artifact was unavailable.
    #[error("required runtime artifact `{0}` was not found")]
    MissingArtifact(String),
    /// Worker exceeded the configured bound.
    #[error("Gradle worker exceeded timeout of {0:?}")]
    Timeout(Duration),
    /// A recording run was stale and cannot be replayed.
    #[error("recording run became stale")]
    StaleRecording,
    /// Workspace content no longer matches the revision captured by a recording.
    #[error("recording revision `{recorded}` does not match current revision `{current}`")]
    ReplayRevision {
        /// Revision stored with the recording.
        recorded: String,
        /// Revision captured immediately before replay.
        current: String,
    },
    /// Fresh execution diverged from the recording.
    #[error(transparent)]
    Replay(#[from] ReplayError),
}

/// Supervised Gradle verifier bound to one worktree/store.
#[derive(Debug)]
pub struct Verifier<'a> {
    layout: &'a WorkspaceLayout,
    store: &'a Store,
    artifacts: RuntimeArtifacts,
    timeout: Duration,
}

impl<'a> Verifier<'a> {
    /// Constructs a verifier using explicit runtime artifacts.
    #[must_use]
    pub fn new(layout: &'a WorkspaceLayout, store: &'a Store, artifacts: RuntimeArtifacts) -> Self {
        Self {
            layout,
            store,
            artifacts,
            timeout: Duration::from_mins(5),
        }
    }

    /// Overrides the worker timeout (primarily for tests and local policy).
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Runs Gradle and ingests whatever structured evidence the worker emitted.
    pub fn verify(
        &self,
        request: &VerificationRequest,
    ) -> Result<VerificationOutcome, VerifierError> {
        self.verify_internal(request, None)
    }

    fn verify_internal(
        &self,
        request: &VerificationRequest,
        replay_recording: Option<&Recording>,
    ) -> Result<VerificationOutcome, VerifierError> {
        validate_artifacts(&self.artifacts)?;
        let before = RevisionSnapshot::capture(&self.layout.root)?;
        let cacheable = replay_recording.is_none()
            && request.mode == VerificationMode::Fast
            && request.selected_tests.is_empty()
            && !request.full
            && request.since.is_none();
        let impactable = request.mode == VerificationMode::Fast
            && request.selected_tests.is_empty()
            && !request.full;
        if cacheable
            && let Some(payload) = self.store.verification_cache(before.revision.as_str())?
        {
            let after = RevisionSnapshot::capture(&self.layout.root)?;
            if after.revision == before.revision {
                let mut outcome: VerificationOutcome = serde_json::from_slice(&payload)?;
                outcome.cached = true;
                outcome.selection = "cached".into();
                return Ok(outcome);
            }
        }
        let mut effective_request = request.clone();
        let mut selection = if !request.selected_tests.is_empty() {
            "explicit"
        } else if request.full {
            "full"
        } else {
            "moduleAll"
        }
        .to_owned();
        if impactable {
            let impacted = match request.since.as_deref() {
                Some(since) if !since.starts_with("rev-") => {
                    self.impacted_tests_since_git(&before, since)?
                }
                since => self.impacted_tests(&before, since)?,
            };
            if let Some(tests) = impacted {
                effective_request.selected_tests = tests;
                selection = "impact".into();
            }
        }
        let run_id = run_id();
        let run_directory = self.layout.state_dir.join("runs").join(&run_id);
        fs::create_dir_all(&run_directory)?;
        let event_file = run_directory.join("events.jsonl");
        let input_file = run_directory.join("inputs.tape");
        let schedule_file = run_directory.join("schedule.tape");
        let log_file = run_directory.join("worker.log");
        let model_directory = run_directory.join("model");
        File::create(&event_file)?;
        let has_replay_schedule = if let Some(recording) = replay_recording {
            write_replay_inputs(recording, &input_file)?;
            write_replay_schedule(recording, &schedule_file)?
        } else {
            File::create(&input_file)?;
            false
        };
        self.store.begin_run(&NewRun {
            id: run_id.clone(),
            revision: before.revision.to_string(),
            source_digest: before.revision.to_string(),
            started_at: Utc::now(),
        })?;

        let worker_files = WorkerFiles {
            event: &event_file,
            input: &input_file,
            replay_inputs: replay_recording.is_some(),
            replay_schedule: has_replay_schedule.then_some(schedule_file.as_path()),
            model_directory: &model_directory,
            log: &log_file,
        };
        let process = self.run_gradle(&effective_request, &worker_files);
        let process = match process {
            Ok(process) => process,
            Err(error) => {
                let _ = self.store.finish_run(
                    &run_id,
                    RunStatus::Error,
                    before.revision.as_str(),
                    Utc::now(),
                );
                return Err(error);
            }
        };
        let ingested = match self.ingest_events(&run_id, &before, &event_file) {
            Ok(ingested) => ingested,
            Err(error) => {
                let _ = self.store.finish_run(
                    &run_id,
                    RunStatus::Error,
                    before.revision.as_str(),
                    Utc::now(),
                );
                return Err(error);
            }
        };
        let after = match RevisionSnapshot::capture(&self.layout.root) {
            Ok(after) => after,
            Err(error) => {
                let _ = self.store.finish_run(
                    &run_id,
                    RunStatus::Error,
                    before.revision.as_str(),
                    Utc::now(),
                );
                return Err(error.into());
            }
        };
        let desired = if process.success {
            RunStatus::Passed
        } else {
            RunStatus::Failed
        };
        let status =
            self.store
                .finish_run(&run_id, desired, after.revision.as_str(), Utc::now())?;
        let exit_code = match status {
            RunStatus::Passed => 0,
            RunStatus::Failed => 1,
            RunStatus::Stale => 4,
            RunStatus::Error | RunStatus::Running => 3,
        };
        let outcome = VerificationOutcome {
            run_id,
            revision: before.revision.to_string(),
            status,
            exit_code,
            tests: ingested.tests,
            failures: ingested.failures,
            events: ingested.event_count,
            capabilities: ingested.capabilities,
            log_file,
            cached: false,
            selection,
            selected_tests: effective_request.selected_tests,
        };
        if cacheable && outcome.status == RunStatus::Passed {
            self.store.save_verification_cache(
                &outcome.revision,
                &outcome.run_id,
                &serde_json::to_vec(&outcome)?,
            )?;
            self.store.save_revision_manifest(
                &outcome.revision,
                &RevisionManifest {
                    file_digests: before.file_digests.clone(),
                    surface_digests: before.surface_digests.clone(),
                },
            )?;
        }
        self.store.prune(Utc::now())?;
        Ok(outcome)
    }

    fn impacted_tests(
        &self,
        current: &RevisionSnapshot,
        requested_baseline: Option<&str>,
    ) -> Result<Option<Vec<String>>, VerifierError> {
        let baseline_revision = if let Some(revision) = requested_baseline {
            Some(revision.to_owned())
        } else {
            self.store.latest_cached_revision()?
        };
        let Some(baseline_revision) = baseline_revision else {
            return Ok(None);
        };
        if baseline_revision == current.revision.as_str() {
            return Ok(None);
        }
        let Some(baseline) = self.store.revision_manifest(&baseline_revision)? else {
            return Ok(None);
        };
        let mut paths = baseline
            .file_digests
            .keys()
            .chain(current.file_digests.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        paths.retain(|path| baseline.file_digests.get(path) != current.file_digests.get(path));
        let mut selected = BTreeSet::new();
        for path in paths {
            let source = implementation_source(&path);
            let existed = baseline.file_digests.contains_key(&path)
                && current.file_digests.contains_key(&path);
            let surface_unchanged =
                baseline.surface_digests.get(&path) == current.surface_digests.get(&path);
            if !source || !existed || !surface_unchanged {
                return Ok(None);
            }
            match self.store.select_impact(&path, &module_for_path(&path))? {
                ImpactSelection::Exact(tests) => selected.extend(tests),
                ImpactSelection::ModuleAll { .. } => return Ok(None),
            }
        }
        Ok((!selected.is_empty()).then(|| selected.into_iter().collect()))
    }

    fn impacted_tests_since_git(
        &self,
        current: &RevisionSnapshot,
        reference: &str,
    ) -> Result<Option<Vec<String>>, VerifierError> {
        if reference.is_empty()
            || reference.len() > 256
            || reference.starts_with('-')
            || !reference
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._/-".contains(character))
        {
            return Ok(None);
        }
        let output = Command::new("git")
            .args([
                "-C",
                self.layout.root.to_string_lossy().as_ref(),
                "diff",
                "--no-ext-diff",
                "--name-status",
                "-z",
                reference,
                "--",
            ])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        let untracked = Command::new("git")
            .args([
                "-C",
                self.layout.root.to_string_lossy().as_ref(),
                "ls-files",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
            ])
            .output()?;
        if !untracked.status.success() {
            return Ok(None);
        }
        for raw_path in untracked.stdout.split(|byte| *byte == 0) {
            if raw_path.is_empty() {
                continue;
            }
            let path = String::from_utf8_lossy(raw_path).replace('\\', "/");
            let absolute = self.layout.root.join(&path);
            let runtime_artifact = absolute == self.artifacts.adapter_jar
                || absolute == self.artifacts.agent_jar
                || absolute == self.artifacts.init_script;
            if !runtime_artifact && (source_input(&path) || build_input(&path)) {
                return Ok(None);
            }
        }
        let fields = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        if fields.len() % 2 != 0 {
            return Ok(None);
        }
        let mut selected = BTreeSet::new();
        for pair in fields.chunks_exact(2) {
            let status = String::from_utf8_lossy(pair[0]);
            let path = String::from_utf8_lossy(pair[1]).replace('\\', "/");
            if status != "M" || !implementation_source(&path) {
                return Ok(None);
            }
            let Some(current_surface) = current.surface_digests.get(&path) else {
                return Ok(None);
            };
            let object = format!("{reference}:{path}");
            let old = Command::new("git")
                .args([
                    "-C",
                    self.layout.root.to_string_lossy().as_ref(),
                    "show",
                    &object,
                ])
                .output()?;
            if !old.status.success()
                || source_surface_digest(Path::new(&path), &old.stdout) != *current_surface
            {
                return Ok(None);
            }
            match self.store.select_impact(&path, &module_for_path(&path))? {
                ImpactSelection::Exact(tests) => selected.extend(tests),
                ImpactSelection::ModuleAll { .. } => return Ok(None),
            }
        }
        Ok((!selected.is_empty()).then(|| selected.into_iter().collect()))
    }

    /// Re-runs one test in full mode and saves an immutable recording.
    pub fn record(&self, test_id: &str) -> Result<Recording, VerifierError> {
        self.record_with_options(test_id, RecordingOptions::default())
    }

    /// Re-runs one test with explicit bounded-input choices and saves an immutable recording.
    pub fn record_with_options(
        &self,
        test_id: &str,
        options: RecordingOptions,
    ) -> Result<Recording, VerifierError> {
        let outcome = self.verify(&VerificationRequest {
            mode: VerificationMode::Full,
            selected_tests: vec![test_id.into()],
            full: false,
            since: None,
            capture_file_io: options.capture_file_io,
        })?;
        if outcome.status == RunStatus::Stale {
            return Err(VerifierError::StaleRecording);
        }
        let events = self
            .store
            .events(&outcome.run_id, None, 100_000, 64 * 1024 * 1024)?
            .items;
        let id = format!(
            "rec-{}",
            short_hash(&format!(
                "recording-v1\0{}\0{}\0{}",
                outcome.revision,
                test_id,
                events
                    .last()
                    .map_or("none", |event| event.state_hash.as_str())
            ))
        );
        let checkpoints = replay_checkpoints(&events);
        let inputs = recorded_inputs(
            &self
                .layout
                .state_dir
                .join("runs")
                .join(&outcome.run_id)
                .join("inputs.tape"),
        )?;
        let mut capabilities = outcome.capabilities;
        if !checkpoints.is_empty() {
            capabilities.supported.push("replayCheckpoints".into());
        }
        let recording = Recording {
            id,
            revision: outcome.revision,
            test_id: test_id.into(),
            backend: "jvm".into(),
            capabilities,
            inputs,
            linux_process: None,
            events,
            checkpoints,
        };
        self.store.save_recording(&recording)?;
        self.store.prune(Utc::now())?;
        Ok(recording)
    }

    /// Re-runs a recorded test in a fresh JVM and verifies its observable prefix through an event.
    pub fn verify_replay_event(
        &self,
        recording: &Recording,
        event_id: &str,
    ) -> Result<String, VerifierError> {
        let current = RevisionSnapshot::capture(&self.layout.root)?;
        if current.revision.as_str() != recording.revision {
            return Err(VerifierError::ReplayRevision {
                recorded: recording.revision.clone(),
                current: current.revision.to_string(),
            });
        }
        let outcome = self.verify_internal(
            &VerificationRequest {
                mode: VerificationMode::Full,
                selected_tests: vec![recording.test_id.clone()],
                full: false,
                since: None,
                capture_file_io: recording
                    .inputs
                    .iter()
                    .any(|input| input.kind.starts_with("io.file.")),
            },
            Some(recording),
        )?;
        if outcome.status == RunStatus::Stale {
            return Err(VerifierError::StaleRecording);
        }
        let replayed = self
            .store
            .events(&outcome.run_id, None, 100_000, 64 * 1024 * 1024)?
            .items;
        verify_replayed_prefix(recording, &replayed, event_id)?;
        Ok(outcome.run_id)
    }

    fn run_gradle(
        &self,
        request: &VerificationRequest,
        files: &WorkerFiles<'_>,
    ) -> Result<ProcessOutcome, VerifierError> {
        if self.layout.root.join("pom.xml").is_file() && !has_gradle_wrapper(&self.layout.root) {
            return self.run_maven(request, files);
        }
        let gradle = build_tool_executable(&self.layout.root, "gradlew", "gradlew.bat", "gradle");
        let log = File::create(files.log)?;
        let error_log = log.try_clone()?;
        let mut command = Command::new(gradle);
        command
            .arg("--no-daemon")
            .arg("--configuration-cache")
            .arg("--init-script")
            .arg(&self.artifacts.init_script)
            .arg("walaruVerify")
            .arg(format!(
                "-Dwalaru.adapterJar={}",
                self.artifacts.adapter_jar.display()
            ))
            .arg(format!(
                "-Dwalaru.agentJar={}",
                self.artifacts.agent_jar.display()
            ))
            .arg(format!(
                "-Dwalaru.workspaceRoot={}",
                self.layout.root.display()
            ))
            .arg(format!("-Dwalaru.eventFile={}", files.event.display()))
            .arg(format!("-Dwalaru.mode={}", request.mode.as_str()))
            .arg(format!(
                "-Dwalaru.modelDirectory={}",
                files.model_directory.display()
            ))
            .current_dir(&self.layout.root)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(error_log));
        if request.capture_file_io {
            command.arg("-Dwalaru.captureFileIo=true");
        }
        if request.mode == VerificationMode::Full {
            command.arg(format!(
                "-D{}={}",
                if files.replay_inputs {
                    "walaru.replayInputFile"
                } else {
                    "walaru.inputFile"
                },
                files.input.display()
            ));
        }
        if let Some(schedule) = files.replay_schedule {
            command.arg(format!(
                "-Dwalaru.replayScheduleFile={}",
                schedule.display()
            ));
        }
        if !request.selected_tests.is_empty() {
            let filters = request
                .selected_tests
                .iter()
                .map(|test| test.replace('#', "."))
                .collect::<Vec<_>>()
                .join(",");
            command.arg(format!("-Dwalaru.tests={filters}"));
        }
        configure_process_group(&mut command);
        self.supervise_logged(command.spawn()?, files.log)
    }

    fn run_maven(
        &self,
        request: &VerificationRequest,
        files: &WorkerFiles<'_>,
    ) -> Result<ProcessOutcome, VerifierError> {
        let maven = build_tool_executable(&self.layout.root, "mvnw", "mvnw.cmd", "mvn");
        let roots = self.layout.root.to_string_lossy();
        let mut fork_arguments = vec![
            format!("-javaagent:{}", self.artifacts.agent_jar.display()),
            format!("-Dwalaru.eventFile={}", files.event.display()),
            format!("-Dwalaru.mode={}", request.mode.as_str()),
            format!("-Dwalaru.classRoots={roots}"),
            "-Dwalaru.projectPath=:".into(),
        ];
        if request.capture_file_io {
            fork_arguments.push("-Dwalaru.captureFileIo=true".into());
        }
        let input_property = (request.mode == VerificationMode::Full).then(|| {
            format!(
                "-D{}={}",
                if files.replay_inputs {
                    "walaru.replayInputFile"
                } else {
                    "walaru.inputFile"
                },
                files.input.display()
            )
        });
        if let Some(property) = &input_property {
            fork_arguments.push(property.clone());
        }
        let schedule_property = files
            .replay_schedule
            .map(|path| format!("-Dwalaru.replayScheduleFile={}", path.display()));
        if let Some(property) = &schedule_property {
            fork_arguments.push(property.clone());
        }

        let log = File::create(files.log)?;
        let error_log = log.try_clone()?;
        let mut command = Command::new(maven);
        command
            .arg("--batch-mode")
            .arg("--no-transfer-progress")
            .arg("test")
            .arg(format!("-DargLine={}", fork_arguments.join(" ")))
            .arg(format!(
                "-Dmaven.test.additionalClasspath={}",
                self.artifacts.agent_jar.display()
            ))
            .arg(format!("-Dwalaru.eventFile={}", files.event.display()))
            .arg(format!("-Dwalaru.mode={}", request.mode.as_str()))
            .arg("-Dsurefire.useModulePath=false")
            .arg("-Dsurefire.failIfNoSpecifiedTests=false")
            .arg("-DforkCount=1")
            .current_dir(&self.layout.root)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(error_log));
        if request.capture_file_io {
            command.arg("-Dwalaru.captureFileIo=true");
        }
        if let Some(property) = input_property {
            command.arg(property);
        }
        if let Some(property) = schedule_property {
            command.arg(property);
        }
        if !request.selected_tests.is_empty() {
            let selectors = request
                .selected_tests
                .iter()
                .map(|test| test.split_once("::").map_or(test.as_str(), |(_, id)| id))
                .collect::<Vec<_>>()
                .join(",");
            command.arg(format!("-Dtest={selectors}"));
        }
        configure_process_group(&mut command);
        self.supervise_logged(command.spawn()?, files.log)
    }

    fn supervise(&self, mut child: std::process::Child) -> Result<ProcessOutcome, VerifierError> {
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(ProcessOutcome {
                    success: status.success(),
                });
            }
            if started.elapsed() >= self.timeout {
                terminate_process_tree(&mut child);
                return Err(VerifierError::Timeout(self.timeout));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn supervise_logged(
        &self,
        child: std::process::Child,
        log_file: &Path,
    ) -> Result<ProcessOutcome, VerifierError> {
        let result = self.supervise(child);
        truncate_file_tail(log_file, MAX_WORKER_LOG_BYTES)?;
        result
    }

    fn ingest_events(
        &self,
        run_id: &str,
        revision: &RevisionSnapshot,
        event_file: &Path,
    ) -> Result<Ingested, VerifierError> {
        if !event_file.is_file() {
            return Ok(Ingested::default());
        }
        let mut reader = BufReader::new(File::open(event_file)?);
        let mut line = Vec::new();
        let mut tests: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut modules: BTreeMap<String, String> = BTreeMap::new();
        let mut coverage: BTreeMap<String, BTreeSet<(String, u32, String)>> = BTreeMap::new();
        let mut dependencies: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        let mut failures = Vec::new();
        let mut last_event: BTreeMap<String, String> = BTreeMap::new();
        let mut threads = BTreeSet::new();
        let mut thread_keys: BTreeMap<u64, String> = BTreeMap::new();
        let mut thread_identity_conflict = false;
        let mut virtual_thread_observed = false;
        let mut coroutine_observed = false;
        let mut unavailable = BTreeMap::new();
        let mut nondeterministic_calls = 0_usize;
        let mut deterministic_inputs = 0_usize;
        let mut supported_file_io_calls = 0_usize;
        let mut recorded_file_inputs = 0_usize;
        let mut unsupported_io_observed = false;
        let mut field_reads = false;
        let mut array_writes = false;
        let mut monitor_order = false;
        let mut volatile_access = false;
        let mut event_count = 0_usize;

        while read_bounded_line(&mut reader, &mut line, MAX_EVENT_LINE_BYTES)? != 0 {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if event_count >= MAX_EVENTS_PER_RUN {
                return Err(VerifierError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "JVM event stream exceeded one million events",
                )));
            }
            let raw: Value = serde_json::from_slice(&line)?;
            // Each Gradle Test worker owns a local counter. The append stream order is the
            // revision-wide cross-index and therefore must be assigned here.
            let sequence = u64::try_from(event_count).unwrap_or(u64::MAX);
            let thread_id = raw.get("threadId").and_then(Value::as_u64).unwrap_or(0);
            threads.insert(thread_id);
            let thread_key = raw.get("threadKey").and_then(Value::as_str).unwrap_or("");
            if !thread_key.is_empty()
                && thread_keys
                    .insert(thread_id, thread_key.into())
                    .is_some_and(|previous| previous != thread_key)
            {
                thread_identity_conflict = true;
            }
            virtual_thread_observed |= raw
                .get("virtualThread")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let test_id = public_test_id(&raw);
            let module = raw
                .get("module")
                .and_then(Value::as_str)
                .unwrap_or(":")
                .to_owned();
            modules.entry(test_id.clone()).or_insert(module);
            let kind_name = raw.get("type").and_then(Value::as_str).unwrap_or("OUTPUT");
            let kind = event_kind(kind_name);
            let path = raw
                .get("path")
                .and_then(Value::as_str)
                .map(|path| resolve_source_path(&self.layout.root, path));
            let line_number = raw
                .get("line")
                .and_then(Value::as_u64)
                .and_then(|line| u32::try_from(line).ok())
                .unwrap_or(1);
            let owner = raw.get("owner").and_then(Value::as_str).unwrap_or("");
            let method = raw.get("method").and_then(Value::as_str).unwrap_or("");
            coroutine_observed |= raw
                .get("descriptor")
                .and_then(Value::as_str)
                .is_some_and(|descriptor| descriptor.contains("kotlin/coroutines/Continuation"));
            let symbol = format!("{}#{method}", owner.replace('/', "."));
            let location = path.as_ref().map(|path| SourceLocation {
                path: path.clone(),
                line: line_number.max(1),
                column: 1,
                symbol: symbol.clone(),
            });
            let identity = EventIdentity {
                revision: revision.revision.clone(),
                run_id: run_id.into(),
                test_id: test_id.clone(),
                sequence,
                thread_id,
            };
            let event = Event {
                id: EventId::new(&identity).to_string(),
                sequence,
                thread_id,
                thread_key: raw
                    .get("threadKey")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                virtual_thread: raw
                    .get("virtualThread")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                coroutine: raw
                    .get("coroutine")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                logical_stack: raw
                    .get("logicalStack")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .take(64)
                    .filter_map(logical_frame)
                    .collect(),
                kind,
                location,
                values: public_event_values(raw.get("values")),
                observations: raw
                    .get("observations")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                state_hash: raw
                    .get("stateHash")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                output_index: raw.get("outputIndex").and_then(Value::as_u64).unwrap_or(0),
            };
            self.store.append_event(run_id, &test_id, &event)?;
            last_event.insert(test_id.clone(), event.id.clone());
            event_count += 1;

            if kind_name == "TEST_START" {
                tests.entry(test_id.clone()).or_insert(None);
            }
            if kind_name == "TEST_FINISH" {
                let raw_status = raw
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let status = match raw_status {
                    "successful" => "passed",
                    "aborted" => "skipped",
                    other => other,
                }
                .to_owned();
                tests.insert(test_id.clone(), Some(status.clone()));
                if status == "failed" {
                    let failure_id = format!(
                        "failure-{}",
                        short_hash(&format!("{run_id}\0{test_id}\0{sequence}"))
                    );
                    self.store.save_failure(&FailureRecord {
                        id: failure_id.clone(),
                        run_id: run_id.into(),
                        test_id: test_id.clone(),
                        exception_type: raw
                            .get("failureType")
                            .and_then(Value::as_str)
                            .unwrap_or("java.lang.AssertionError")
                            .into(),
                        message: raw
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("test failed; inspect trace and worker log")
                            .chars()
                            .take(512)
                            .collect(),
                        event_id: last_event.get(&test_id).cloned(),
                        frames: raw
                            .get("frames")
                            .and_then(Value::as_array)
                            .map(|frames| {
                                frames
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .take(64)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })?;
                    failures.push(failure_id);
                }
            }
            if kind_name == "LINE"
                && let Some(path) = path
            {
                coverage.entry(test_id.clone()).or_default().insert((
                    path.clone(),
                    line_number.max(1),
                    symbol.clone(),
                ));
                dependencies
                    .entry(test_id.clone())
                    .or_default()
                    .insert((path, "method".into()));
            }
            if kind_name == "CALL" {
                if nondeterministic_boundary(&raw) {
                    nondeterministic_calls += 1;
                }
                if supported_file_io_boundary(&raw) {
                    supported_file_io_calls += 1;
                } else if io_boundary(&raw) {
                    unsupported_io_observed = true;
                }
                classify_boundary(&raw, &mut unavailable);
            }
            if kind_name == "INPUT" {
                let file_input = raw
                    .get("values")
                    .and_then(|values| values.get("kind"))
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.starts_with("io.file."));
                if file_input {
                    recorded_file_inputs += 1;
                } else {
                    deterministic_inputs += 1;
                }
            }
            if kind_name == "READ"
                && raw
                    .get("values")
                    .and_then(|values| values.get("targetKind"))
                    .and_then(Value::as_str)
                    == Some("field")
            {
                field_reads = true;
            }
            if kind_name == "WRITE"
                && raw
                    .get("values")
                    .and_then(|values| values.get("targetKind"))
                    .and_then(Value::as_str)
                    == Some("array")
            {
                array_writes = true;
            }
            if kind_name == "MONITOR" {
                monitor_order = true;
            }
            if matches!(kind_name, "READ" | "WRITE")
                && (raw.get("volatile").and_then(Value::as_bool) == Some(true)
                    || raw
                        .get("values")
                        .and_then(|values| values.get("volatile"))
                        .and_then(Value::as_bool)
                        == Some(true))
            {
                volatile_access = true;
            }
            if kind_name == "CAPABILITY"
                && raw.get("available").and_then(Value::as_bool) == Some(false)
            {
                let capability = raw
                    .get("capability")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let reason = raw
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("worker reported an unavailable capability");
                unavailable.insert(capability.into(), reason.into());
            }
        }

        for (test_id, status) in &tests {
            self.store.upsert_test(&TestRecord {
                id: test_id.clone(),
                display_name: test_id.clone(),
                module: modules.get(test_id).cloned().unwrap_or_else(|| ":".into()),
                last_status: status.clone(),
                last_failure_id: None,
            })?;
            let records = coverage
                .remove(test_id)
                .unwrap_or_default()
                .into_iter()
                .map(|(path, line, symbol)| CoverageRecord {
                    test_id: test_id.clone(),
                    path,
                    line,
                    symbol,
                })
                .collect::<Vec<_>>();
            self.store.replace_coverage(test_id, &records)?;
            let records = dependencies
                .remove(test_id)
                .unwrap_or_default()
                .into_iter()
                .map(|(subject, kind)| Dependency { subject, kind })
                .collect::<Vec<_>>();
            self.store.replace_dependencies(test_id, &records)?;
        }
        let distinct_thread_keys = thread_keys.values().collect::<BTreeSet<_>>();
        let scheduled_threads = threads.len() > 1
            && !thread_identity_conflict
            && thread_keys.len() == threads.len()
            && distinct_thread_keys.len() == threads.len()
            && distinct_thread_keys
                .iter()
                .all(|key| !key.ends_with(":<unnamed>"));
        if threads.len() > 1 && !scheduled_threads {
            unavailable.insert(
                "threads".into(),
                format!(
                    "{} JVM threads emitted events but lacked unique stable names",
                    threads.len()
                ),
            );
        }
        if nondeterministic_calls > 0 && deterministic_inputs == nondeterministic_calls {
            unavailable.remove("nondeterminism");
        }
        if supported_file_io_calls > 0
            && recorded_file_inputs == supported_file_io_calls
            && !unsupported_io_observed
        {
            unavailable.remove("io");
        }
        let mut supported = vec!["line".into(), "call".into(), "write".into()];
        if tests.len() <= 1 {
            supported.push("singleTest".into());
        } else {
            supported.push("multipleTests".into());
        }
        if threads.len() <= 1 {
            supported.push("singleThread".into());
        }
        if scheduled_threads {
            supported.extend(["threads".into(), "threadSchedule".into()]);
            if coroutine_observed {
                supported.push("coroutineSchedule".into());
            }
        }
        if virtual_thread_observed {
            supported.push("virtualThreads".into());
        }
        if unavailable.is_empty() {
            supported.push("pureJvm".into());
        }
        if deterministic_inputs > 0 && deterministic_inputs == nondeterministic_calls {
            supported.push("deterministicInputs".into());
        }
        if recorded_file_inputs > 0
            && recorded_file_inputs == supported_file_io_calls
            && !unsupported_io_observed
        {
            supported.push("fileInputs".into());
        }
        if field_reads {
            supported.push("fieldReads".into());
        }
        if array_writes {
            supported.push("arrayWrites".into());
        }
        if monitor_order {
            supported.push("monitorOrder".into());
        }
        if volatile_access {
            supported.push("volatileAccess".into());
        }
        if (field_reads || array_writes || monitor_order || volatile_access) && scheduled_threads {
            supported.push("memorySchedule".into());
        }
        Ok(Ingested {
            tests: tests.into_keys().collect(),
            failures,
            event_count,
            capabilities: CapabilityManifest {
                backend: "jvm".into(),
                completeness: if unavailable.is_empty() {
                    Completeness::Complete
                } else {
                    Completeness::Partial
                },
                supported,
                unavailable,
            },
        })
    }
}

#[derive(Debug)]
struct ProcessOutcome {
    success: bool,
}

#[derive(Clone, Copy, Debug)]
struct WorkerFiles<'a> {
    event: &'a Path,
    input: &'a Path,
    replay_inputs: bool,
    replay_schedule: Option<&'a Path>,
    model_directory: &'a Path,
    log: &'a Path,
}

#[derive(Debug)]
struct Ingested {
    tests: Vec<String>,
    failures: Vec<String>,
    event_count: usize,
    capabilities: CapabilityManifest,
}

impl Default for Ingested {
    fn default() -> Self {
        Self {
            tests: Vec::new(),
            failures: Vec::new(),
            event_count: 0,
            capabilities: CapabilityManifest {
                backend: "jvm".into(),
                completeness: Completeness::Partial,
                supported: Vec::new(),
                unavailable: [("events".into(), "worker emitted no event stream".into())]
                    .into_iter()
                    .collect(),
            },
        }
    }
}

fn has_gradle_wrapper(root: &Path) -> bool {
    root.join("gradlew").is_file() || root.join("gradlew.bat").is_file()
}

fn build_tool_executable(
    root: &Path,
    unix_wrapper: &str,
    windows_wrapper: &str,
    fallback: &str,
) -> PathBuf {
    #[cfg(windows)]
    if root.join(windows_wrapper).is_file() {
        return root.join(windows_wrapper);
    }
    if root.join(unix_wrapper).is_file() {
        return root.join(unix_wrapper);
    }
    #[cfg(not(windows))]
    if root.join(windows_wrapper).is_file() {
        return root.join(windows_wrapper);
    }
    PathBuf::from(fallback)
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = i32::try_from(child.id()).unwrap_or(i32::MAX);
        let _ = killpg(Pid::from_raw(pid), Signal::SIGTERM);
        thread::sleep(Duration::from_millis(100));
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
    max_bytes: usize,
) -> std::io::Result<usize> {
    output.clear();
    loop {
        let (consumed, finished) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(output.len());
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(available.len(), |index| index + 1);
            if output.len().saturating_add(consumed) > max_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("JVM event line exceeds {max_bytes} bytes"),
                ));
            }
            output.extend_from_slice(&available[..consumed]);
            (consumed, newline.is_some())
        };
        reader.consume(consumed);
        if finished {
            return Ok(output.len());
        }
    }
}

fn truncate_file_tail(path: &Path, max_bytes: u64) -> std::io::Result<()> {
    let size = fs::metadata(path)?.len();
    if size <= max_bytes {
        return Ok(());
    }
    let mut input = File::open(path)?;
    input.seek(SeekFrom::Start(size - max_bytes))?;
    let mut tail = Vec::with_capacity(usize::try_from(max_bytes).unwrap_or(8 * 1024 * 1024));
    input.take(max_bytes).read_to_end(&mut tail)?;
    fs::write(path, tail)
}

fn validate_artifacts(artifacts: &RuntimeArtifacts) -> Result<(), VerifierError> {
    for path in [
        &artifacts.adapter_jar,
        &artifacts.agent_jar,
        &artifacts.init_script,
    ] {
        if !path.is_file() {
            return Err(VerifierError::MissingArtifact(path.display().to_string()));
        }
    }
    Ok(())
}

fn artifact_path(name: &str, candidates: &[PathBuf]) -> Result<PathBuf, VerifierError> {
    if let Some(path) = std::env::var_os(name).map(PathBuf::from) {
        return path
            .is_file()
            .then_some(path)
            .ok_or_else(|| VerifierError::MissingArtifact(name.into()));
    }
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .ok_or_else(|| VerifierError::MissingArtifact(name.into()))
}

fn run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("run-{}-{nanos}", std::process::id())
}

fn public_test_id(raw: &Value) -> String {
    if let Some(name) = raw.get("testName").and_then(Value::as_str)
        && name.contains('#')
    {
        return name.into();
    }
    let unique = raw
        .get("testId")
        .and_then(Value::as_str)
        .unwrap_or("unknown-test");
    let class = unique
        .split("[class:")
        .nth(1)
        .and_then(|part| part.split(']').next());
    let method = unique
        .split("[method:")
        .nth(1)
        .and_then(|part| part.split(']').next())
        .map(|method| method.split('(').next().unwrap_or(method));
    match (class, method) {
        (Some(class), Some(method)) => format!("{class}#{method}"),
        _ => raw
            .get("testName")
            .and_then(Value::as_str)
            .unwrap_or(unique)
            .into(),
    }
}

fn logical_frame(raw: &Value) -> Option<LogicalFrame> {
    Some(LogicalFrame {
        class_name: raw.get("className")?.as_str()?.into(),
        method: raw.get("method")?.as_str()?.into(),
        path: raw.get("path")?.as_str()?.replace('\\', "/"),
        line: raw
            .get("line")?
            .as_u64()
            .and_then(|line| u32::try_from(line).ok())?
            .max(1),
    })
}

fn event_kind(value: &str) -> EventKind {
    match value {
        "TEST_START" => EventKind::TestStart,
        "TEST_FINISH" => EventKind::TestFinish,
        "LINE" => EventKind::Line,
        "WRITE" => EventKind::Write,
        "READ" => EventKind::Read,
        "MONITOR" => EventKind::Monitor,
        "INPUT" => EventKind::Input,
        "CHECKPOINT" => EventKind::Checkpoint,
        "CAPTURE" => EventKind::Capture,
        "NOTE" => EventKind::Note,
        "SPAN_START" => EventKind::SpanStart,
        "SPAN_VALUE" => EventKind::SpanValue,
        "SPAN_END" => EventKind::SpanEnd,
        "CALL" | "METHOD_ENTER" | "METHOD_EXIT" => EventKind::Call,
        _ => EventKind::Output,
    }
}

fn public_event_values(raw: Option<&Value>) -> Value {
    let mut values = raw.cloned().unwrap_or_else(|| json!({}));
    let Some(object) = values.as_object_mut() else {
        return values;
    };
    let redacted = object
        .get("redacted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sensitive = object
        .get("sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if redacted || sensitive {
        object.insert(
            "value".into(),
            Value::String(sensitive_placeholder(object).unwrap_or_else(|| "<redacted>".into())),
        );
        object.remove("encoded");
    }
    values
}

fn sensitive_placeholder(values: &serde_json::Map<String, Value>) -> Option<String> {
    if !values
        .get("sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let bytes = values
        .get("value")?
        .as_str()?
        .strip_prefix("<redacted:file-input ")?
        .strip_suffix(" bytes>")?
        .parse::<usize>()
        .ok()?;
    if bytes > MAX_EVENT_LINE_BYTES {
        return None;
    }
    Some(format!("<redacted:file-input {bytes} bytes>"))
}

fn write_replay_inputs(recording: &Recording, path: &Path) -> Result<(), VerifierError> {
    let mut tape = String::new();
    if !recording.inputs.is_empty() {
        for input in &recording.inputs {
            append_tape_entry(&mut tape, &input.kind, &input.encoded)?;
        }
        fs::write(path, tape)?;
        return Ok(());
    }
    for event in recording
        .events
        .iter()
        .filter(|event| event.kind == EventKind::Input)
    {
        let kind = event
            .values
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        let encoded = event
            .values
            .get("encoded")
            .and_then(Value::as_str)
            .unwrap_or("");
        append_tape_entry(&mut tape, kind, encoded)?;
    }
    fs::write(path, tape)?;
    Ok(())
}

fn recorded_inputs(path: &Path) -> Result<Vec<RecordedInput>, VerifierError> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let mut inputs = Vec::new();
    for line in fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.is_empty())
    {
        let Some((kind, encoded)) = line.split_once('\t') else {
            return Err(invalid_input_tape());
        };
        let mut validated = String::new();
        append_tape_entry(&mut validated, kind, encoded)?;
        inputs.push(RecordedInput {
            kind: kind.into(),
            encoded: encoded.into(),
            sensitive: kind.starts_with("io."),
        });
    }
    Ok(inputs)
}

fn append_tape_entry(tape: &mut String, kind: &str, encoded: &str) -> Result<(), VerifierError> {
    if kind.is_empty()
        || encoded.is_empty()
        || kind.contains(['\t', '\n', '\r'])
        || encoded.contains(['\t', '\n', '\r'])
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err(invalid_input_tape());
    }
    tape.push_str(kind);
    tape.push('\t');
    tape.push_str(encoded);
    tape.push('\n');
    Ok(())
}

fn invalid_input_tape() -> VerifierError {
    VerifierError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "recording contains a malformed deterministic input",
    ))
}

fn write_replay_schedule(recording: &Recording, path: &Path) -> Result<bool, VerifierError> {
    let mut schedule = String::new();
    for event in &recording.events {
        if event.thread_key.is_empty() {
            continue;
        }
        let category = match event.kind {
            EventKind::TestStart => "TEST_START",
            EventKind::Line => "LINE",
            EventKind::Call => "CALL",
            EventKind::Write => "WRITE",
            EventKind::Read => "READ",
            EventKind::Monitor => "MONITOR",
            EventKind::TestFinish => "TEST_FINISH",
            EventKind::Output => "OUTPUT",
            EventKind::Input => "INPUT",
            EventKind::Checkpoint => "CHECKPOINT",
            EventKind::Capture => "CAPTURE",
            EventKind::Note => "NOTE",
            EventKind::SpanStart => "SPAN_START",
            EventKind::SpanValue => "SPAN_VALUE",
            EventKind::SpanEnd => "SPAN_END",
        };
        schedule.push_str(category);
        schedule.push('\t');
        schedule.push_str(&hex::encode(event.thread_key.as_bytes()));
        schedule.push('\n');
    }
    fs::write(path, &schedule)?;
    Ok(!schedule.is_empty())
}

fn resolve_source_path(workspace: &Path, raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    if workspace.join(&normalized).is_file() {
        return normalized;
    }
    let file_name = Path::new(&normalized).file_name();
    let mut matches = Vec::new();
    collect_named_files(&workspace.join("src"), file_name, &mut matches);
    if matches.len() == 1 {
        return matches[0]
            .strip_prefix(workspace)
            .unwrap_or(&matches[0])
            .to_string_lossy()
            .replace('\\', "/");
    }
    normalized
}

fn collect_named_files(
    directory: &Path,
    name: Option<&std::ffi::OsStr>,
    output: &mut Vec<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_named_files(&path, name, output);
        } else if path.file_name() == name {
            output.push(path);
        }
    }
}

fn classify_boundary(raw: &Value, unavailable: &mut BTreeMap<String, String>) {
    let owner = raw.get("targetOwner").and_then(Value::as_str).unwrap_or("");
    let method = raw
        .get("targetMethod")
        .and_then(Value::as_str)
        .unwrap_or("");
    if owner.starts_with("java/io/")
        || owner.starts_with("java/nio/file/")
        || owner.starts_with("java/net/")
    {
        unavailable.insert(
            "io".into(),
            format!("external boundary {owner}#{method} observed"),
        );
    }
    if owner == "java/lang/ProcessBuilder" || owner == "java/lang/Runtime" && method == "exec" {
        unavailable.insert(
            "subprocess".into(),
            format!("subprocess boundary {owner}#{method} observed"),
        );
    }
    if owner == "java/lang/System" && method == "getenv" {
        unavailable.insert(
            "environment".into(),
            "environment variables are redacted and not recorded for replay".into(),
        );
    }
    if owner == "java/lang/System" && method == "getProperty" {
        unavailable.insert(
            "systemProperties".into(),
            "system property reads are redacted and not recorded for replay".into(),
        );
    }
    if nondeterministic_boundary(raw) {
        unavailable.insert(
            "nondeterminism".into(),
            format!("unrecorded nondeterministic input {owner}#{method}"),
        );
    }
    if (owner == "java/lang/System" && matches!(method, "load" | "loadLibrary"))
        || (owner == "java/lang/Runtime" && matches!(method, "load" | "loadLibrary"))
    {
        unavailable.insert(
            "native".into(),
            format!("native library boundary {owner}#{method} observed"),
        );
    }
}

fn io_boundary(raw: &Value) -> bool {
    raw.get("targetOwner")
        .and_then(Value::as_str)
        .is_some_and(|owner| {
            owner.starts_with("java/io/")
                || owner.starts_with("java/nio/file/")
                || owner.starts_with("java/net/")
        })
}

fn supported_file_io_boundary(raw: &Value) -> bool {
    raw.get("targetOwner").and_then(Value::as_str) == Some("java/nio/file/Files")
        && raw
            .get("targetMethod")
            .and_then(Value::as_str)
            .is_some_and(|method| matches!(method, "readAllBytes" | "readString"))
}

fn nondeterministic_boundary(raw: &Value) -> bool {
    let owner = raw.get("targetOwner").and_then(Value::as_str).unwrap_or("");
    let method = raw
        .get("targetMethod")
        .and_then(Value::as_str)
        .unwrap_or("");
    (owner == "java/lang/System" && matches!(method, "currentTimeMillis" | "nanoTime"))
        || (owner.starts_with("java/time/") && method == "now")
        || ((owner.starts_with("java/util/Random")
            || owner == "java/util/SplittableRandom"
            || owner.starts_with("java/util/random/")
            || owner == "java/util/concurrent/ThreadLocalRandom"
            || owner == "java/security/SecureRandom")
            && (method.starts_with("next") || method == "generateSeed"))
        || (owner == "java/util/UUID" && method == "randomUUID")
        || (owner == "java/lang/Math" && method == "random")
}

fn implementation_source(path: &str) -> bool {
    let path = path.replace('\\', "/");
    (path.starts_with("src/main/") || path.contains("/src/main/"))
        && matches!(
            Path::new(&path)
                .extension()
                .and_then(|value| value.to_str()),
            Some("java" | "kt")
        )
}

fn source_input(path: &str) -> bool {
    let path = path.replace('\\', "/");
    path.starts_with("src/") || path.contains("/src/")
}

fn module_for_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let Some((prefix, _)) = normalized.split_once("/src/") else {
        return ":".into();
    };
    if prefix.is_empty() {
        ":".into()
    } else {
        format!(":{}", prefix.trim_matches('/').replace('/', ":"))
    }
}

fn build_input(path: &str) -> bool {
    matches!(
        Path::new(path).file_name().and_then(|value| value.to_str()),
        Some(
            "build.gradle"
                | "build.gradle.kts"
                | "settings.gradle"
                | "settings.gradle.kts"
                | "gradle.properties"
                | "libs.versions.toml"
                | "gradle-wrapper.properties"
                | "pom.xml"
                | "maven.config"
                | "jvm.config"
                | "extensions.xml"
                | "maven-wrapper.properties"
                | "walaru.toml"
        )
    )
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..12])
}

fn replay_checkpoints(events: &[Event]) -> Vec<ReplayCheckpoint> {
    events
        .iter()
        .enumerate()
        .filter(|(index, _)| *index == 0 || *index % 256 == 0 || *index + 1 == events.len())
        .map(|(_, event)| ReplayCheckpoint {
            sequence: event.sequence,
            event_id: event.id.clone(),
            state_hash: event.state_hash.clone(),
            output_index: event.output_index,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    #[test]
    fn event_reader_rejects_a_line_before_allocating_past_its_limit() {
        let mut reader = BufReader::new(Cursor::new(b"123456789\nnext\n"));
        let mut line = Vec::new();

        let error = read_bounded_line(&mut reader, &mut line, 8).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(line.len() <= 8);
    }

    #[test]
    fn unrecorded_external_and_nondeterministic_calls_are_capability_failures() {
        for (owner, method, capability) in [
            ("java/lang/System", "getenv", "environment"),
            ("java/lang/System", "getProperty", "systemProperties"),
            ("java/time/Instant", "now", "nondeterminism"),
            (
                "java/util/concurrent/ThreadLocalRandom",
                "nextInt",
                "nondeterminism",
            ),
            ("java/security/SecureRandom", "nextBytes", "nondeterminism"),
            ("java/util/SplittableRandom", "nextInt", "nondeterminism"),
            (
                "java/util/random/RandomGenerator",
                "nextDouble",
                "nondeterminism",
            ),
            ("java/lang/System", "loadLibrary", "native"),
        ] {
            let mut unavailable = BTreeMap::new();
            classify_boundary(
                &json!({"targetOwner": owner, "targetMethod": method}),
                &mut unavailable,
            );
            assert!(
                unavailable.contains_key(capability),
                "{owner}#{method} did not mark {capability}: {unavailable:?}"
            );
        }
        assert!(!nondeterministic_boundary(
            &json!({"targetOwner": "java/util/Random", "targetMethod": "<init>"})
        ));
    }

    #[test]
    fn maven_and_gradle_configuration_are_conservative_build_inputs() {
        for path in [
            "pom.xml",
            "module/pom.xml",
            ".mvn/maven.config",
            ".mvn/jvm.config",
            ".mvn/extensions.xml",
            ".mvn/wrapper/maven-wrapper.properties",
            "build.gradle.kts",
        ] {
            assert!(build_input(path), "{path} was not treated as a build input");
        }
        assert!(!build_input("src/main/java/demo/App.java"));
    }

    #[test]
    fn public_events_enforce_worker_redaction_markers() {
        assert_eq!(
            public_event_values(Some(&json!({
                "name": "apiToken",
                "value": "must-not-leak",
                "redacted": true,
            }))),
            json!({"name": "apiToken", "value": "<redacted>", "redacted": true}),
        );
        assert_eq!(
            public_event_values(Some(&json!({
                "kind": "io.file.readString",
                "value": "<redacted:file-input 14 bytes>",
                "encoded": "bXVzdC1ub3QtbGVhaw==",
                "sensitive": true,
            }))),
            json!({
                "kind": "io.file.readString",
                "value": "<redacted:file-input 14 bytes>",
                "sensitive": true,
            }),
        );
        assert_eq!(
            public_event_values(Some(&json!({
                "value": "<redacted:token=must-not-leak>",
                "redacted": true,
            }))),
            json!({"value": "<redacted>", "redacted": true}),
        );
    }
}
