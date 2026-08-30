//! Public replay boundary and the initial single-thread JVM implementation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

use crate::protocol::{CapabilityManifest, Completeness};

/// A revision-bound execution recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recording {
    /// Recording ID.
    pub id: String,
    /// Source revision.
    pub revision: String,
    /// Single selected test for the M1 backend.
    pub test_id: String,
    /// Backend that created the recording.
    pub backend: String,
    /// Captured and missing capabilities.
    pub capabilities: CapabilityManifest,
    /// Ordered replay-only input tape entries; query endpoints never project this field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<RecordedInput>,
    /// Optional `rr` trace and JVM/native event cross-index for Linux process replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linux_process: Option<LinuxProcessReplayArtifact>,
    /// Ordered cross-indexed events.
    pub events: Vec<Event>,
    /// Sparse verified anchors used by replay/checkpoint adapters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<ReplayCheckpoint>,
}

/// One private deterministic-input tape entry persisted inside a recording blob.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordedInput {
    /// Ordered input kind, including a non-reversible path fingerprint where applicable.
    pub kind: String,
    /// Base64 payload consumed only by a fresh replay worker.
    pub encoded: String,
    /// Whether public event queries must expose only a redacted summary.
    pub sensitive: bool,
}

/// Immutable observable-state anchor within a recording.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayCheckpoint {
    /// Anchored event sequence.
    pub sequence: u64,
    /// Anchored event identity.
    pub event_id: String,
    /// Captured observable-state digest.
    pub state_hash: String,
    /// Captured output position.
    pub output_index: u64,
}

/// Experimental whole-process trace metadata kept separate from JVM guarantees.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinuxProcessReplayArtifact {
    /// Absolute or recording-relative `rr` trace directory.
    pub trace_directory: String,
    /// Walaru event ID to `rr` event number cross-index.
    pub event_index: BTreeMap<String, u64>,
}

/// Shell-free command plan for navigating an `rr` trace to a Walaru event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinuxReplayPlan {
    /// Explicit `rr` executable.
    pub executable: String,
    /// Argument vector passed directly to the executable.
    pub argv: Vec<String>,
    /// Cross-indexed Walaru event selected by the reverse request.
    pub event: Event,
    /// Always false until a native/JVM observable-state proof is captured and compared.
    pub verified: bool,
}

/// Trace event used by both replay backends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    /// Stable event ID.
    pub id: String,
    /// Ordering within the run.
    pub sequence: u64,
    /// JVM/native cross-index thread ID.
    pub thread_id: u64,
    /// Stable logical thread key used across fresh JVMs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thread_key: String,
    /// Whether the event was emitted by a virtual thread.
    #[serde(default)]
    pub virtual_thread: bool,
    /// Whether this boundary belongs to a Kotlin coroutine state machine.
    #[serde(default)]
    pub coroutine: bool,
    /// Bounded Kotlin-level logical stack for coroutine events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logical_stack: Vec<LogicalFrame>,
    /// Reversible event category.
    pub kind: EventKind,
    /// Kotlin/Java logical source location when available.
    pub location: Option<SourceLocation>,
    /// Safely captured values without invoking user getters or `toString()`.
    pub values: Value,
    /// Deterministic state digest at this event.
    pub state_hash: String,
    /// Output stream position.
    pub output_index: u64,
}

/// One bounded Kotlin/Java logical stack frame.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogicalFrame {
    /// Logical class or generated continuation owner.
    pub class_name: String,
    /// Source-level method name.
    pub method: String,
    /// Logical source path.
    pub path: String,
    /// One-based source line.
    pub line: u32,
}

/// Events that can act as reverse-step boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    /// Test lifecycle start.
    TestStart,
    /// Source line boundary.
    Line,
    /// Call boundary.
    Call,
    /// Field/array write boundary.
    Write,
    /// Field/array read boundary.
    Read,
    /// Explicit or synchronized-method monitor boundary.
    Monitor,
    /// Test lifecycle end.
    TestFinish,
    /// Captured standard output/error fragment.
    Output,
    /// Recorded deterministic input such as time, random, or UUID.
    Input,
    /// Logical replay checkpoint.
    Checkpoint,
}

/// Kotlin- or Java-level source location.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceLocation {
    /// Workspace-relative source path.
    pub path: String,
    /// One-based line.
    pub line: u32,
    /// One-based column.
    pub column: u32,
    /// Logical source symbol.
    pub symbol: String,
}

/// Requested reverse operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReverseRequest {
    /// Event at which reverse search starts, exclusive.
    pub from_event_id: String,
    /// Boundary type for a single reverse step.
    pub step: Option<StepKind>,
    /// Optional exact source target for reverse-continue.
    pub until: Option<SourceTarget>,
    /// Optional exact field (`owner#field`) or array slot (`array[index]`) watchpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<String>,
}

/// Reverse step category.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    /// Previous source line event.
    Line,
    /// Previous call event.
    Call,
    /// Previous write event.
    Write,
}

/// Workspace-relative path and one-based line target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceTarget {
    /// Workspace-relative source path.
    pub path: String,
    /// One-based line.
    pub line: u32,
}

/// Successful reverse result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReverseOutcome {
    /// Backend used for replay.
    pub backend: String,
    /// Matching state event.
    pub event: Event,
    /// Whether state/output hashes were verified.
    pub verified: bool,
}

/// Replay failure with an explicit capability or consistency reason.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ReplayError {
    /// Recording cannot make the requested exactness claim.
    #[error("recording lacks replay capability `{capability}`: {reason}")]
    UnsupportedCapability {
        /// Missing capability.
        capability: String,
        /// Diagnostic reason.
        reason: String,
    },
    /// The starting event is not part of this recording.
    #[error("event `{0}` is not present in the recording")]
    UnknownEvent(String),
    /// No earlier event matches the requested boundary.
    #[error("no matching event exists before `{0}`")]
    NoPreviousEvent(String),
    /// Request did not select a step or target.
    #[error("reverse request must specify exactly one of step or until")]
    InvalidRequest,
    /// Fresh execution stopped matching the recorded observable state.
    #[error("fresh replay diverged at event sequence {sequence}: {reason}")]
    Diverged {
        /// Original event sequence at the first mismatch.
        sequence: u64,
        /// Human-readable mismatch category.
        reason: String,
    },
}

/// Common contract implemented by JVM and Linux process replay engines.
pub trait ReplayBackend {
    /// Static capabilities available on this host.
    fn capabilities(&self) -> CapabilityManifest;

    /// Re-executes or navigates to the requested earlier state.
    fn reverse(
        &self,
        recording: &Recording,
        request: &ReverseRequest,
    ) -> Result<ReverseOutcome, ReplayError>;
}

/// Initial deterministic single-thread, pure-JVM replay backend.
#[derive(Debug, Default)]
pub struct JvmReplayBackend;

impl ReplayBackend for JvmReplayBackend {
    fn capabilities(&self) -> CapabilityManifest {
        CapabilityManifest {
            backend: "jvm".into(),
            completeness: Completeness::Complete,
            supported: vec![
                "singleThread".into(),
                "singleTest".into(),
                "pureJvm".into(),
                "line".into(),
                "call".into(),
                "write".into(),
                "threads".into(),
                "threadSchedule".into(),
                "virtualThreads".into(),
                "deterministicInputs".into(),
                "fieldReads".into(),
                "arrayWrites".into(),
                "monitorOrder".into(),
                "volatileAccess".into(),
                "writeWatchpoints".into(),
                "reverseContinue".into(),
                "replayCheckpoints".into(),
                "boundedFileInputsOptIn".into(),
            ],
            unavailable: [
                ("native".into(), "JNI inputs are not recorded".into()),
                (
                    "io".into(),
                    "network and unbounded file I/O are not recorded; bounded Files reads require explicit opt-in".into(),
                ),
                (
                    "subprocess".into(),
                    "child processes are not recorded".into(),
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    fn reverse(
        &self,
        recording: &Recording,
        request: &ReverseRequest,
    ) -> Result<ReverseOutcome, ReplayError> {
        validate_recording(recording)?;
        if request.step.is_some() == request.until.is_some() {
            return Err(ReplayError::InvalidRequest);
        }
        if request.watch.is_some()
            && (request.step != Some(StepKind::Write)
                || !request.watch.as_deref().is_some_and(valid_watchpoint))
        {
            return Err(ReplayError::InvalidRequest);
        }
        let start = recording
            .events
            .iter()
            .position(|event| event.id == request.from_event_id)
            .ok_or_else(|| ReplayError::UnknownEvent(request.from_event_id.clone()))?;
        let event = recording.events[..start]
            .iter()
            .rev()
            .find(|event| matches_request(event, request))
            .cloned()
            .ok_or_else(|| ReplayError::NoPreviousEvent(request.from_event_id.clone()))?;
        Ok(ReverseOutcome {
            backend: "jvm".into(),
            event,
            verified: false,
        })
    }
}

/// Verifies that a fresh JVM execution matches every observable event through a target event.
///
/// Event IDs and raw thread IDs are intentionally excluded because they are bound to a distinct
/// run. A single-thread recording is compared by event order instead.
pub fn verify_replayed_prefix(
    recording: &Recording,
    replayed: &[Event],
    target_event_id: &str,
) -> Result<(), ReplayError> {
    validate_recording(recording)?;
    let target = recording
        .events
        .iter()
        .position(|event| event.id == target_event_id)
        .ok_or_else(|| ReplayError::UnknownEvent(target_event_id.into()))?;

    for (index, original) in recording.events[..=target].iter().enumerate() {
        let Some(fresh) = replayed.get(index) else {
            return Err(ReplayError::Diverged {
                sequence: original.sequence,
                reason: "fresh execution ended before the target event".into(),
            });
        };
        let reason = if original.kind != fresh.kind {
            Some("event kind changed")
        } else if original.location != fresh.location {
            Some("source location changed")
        } else if original.thread_key != fresh.thread_key
            || original.virtual_thread != fresh.virtual_thread
        {
            Some("logical thread changed")
        } else if original.coroutine != fresh.coroutine
            || original.logical_stack != fresh.logical_stack
        {
            Some("coroutine logical stack changed")
        } else if original.values != fresh.values {
            Some("captured runtime values changed")
        } else if original.state_hash != fresh.state_hash {
            Some("state hash changed")
        } else if original.output_index != fresh.output_index {
            Some("output position changed")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(ReplayError::Diverged {
                sequence: original.sequence,
                reason: reason.into(),
            });
        }
    }
    Ok(())
}

/// Experimental whole-process boundary. `rr` is optional and never implied.
#[derive(Debug, Default)]
pub struct LinuxProcessReplayBackend {
    rr_executable: Option<String>,
}

impl LinuxProcessReplayBackend {
    /// Creates a backend from explicit capability detection.
    #[must_use]
    pub fn new(rr_available: bool) -> Self {
        Self {
            rr_executable: rr_available.then(|| "rr".into()),
        }
    }

    /// Creates an adapter using an explicitly resolved `rr` executable.
    #[must_use]
    pub fn with_rr_executable(executable: impl Into<String>) -> Self {
        Self {
            rr_executable: Some(executable.into()),
        }
    }

    /// Resolves a reverse request and its Walaru/`rr` event cross-index into direct argv.
    pub fn plan_reverse(
        &self,
        recording: &Recording,
        request: &ReverseRequest,
    ) -> Result<LinuxReplayPlan, ReplayError> {
        let executable =
            self.rr_executable
                .as_ref()
                .ok_or_else(|| ReplayError::UnsupportedCapability {
                    capability: "rr".into(),
                    reason: "rr executable was not detected".into(),
                })?;
        if request.step.is_some() == request.until.is_some() {
            return Err(ReplayError::InvalidRequest);
        }
        if request.watch.is_some()
            && (request.step != Some(StepKind::Write)
                || !request.watch.as_deref().is_some_and(valid_watchpoint))
        {
            return Err(ReplayError::InvalidRequest);
        }
        let start = recording
            .events
            .iter()
            .position(|event| event.id == request.from_event_id)
            .ok_or_else(|| ReplayError::UnknownEvent(request.from_event_id.clone()))?;
        let event = recording.events[..start]
            .iter()
            .rev()
            .find(|event| matches_request(event, request))
            .cloned()
            .ok_or_else(|| ReplayError::NoPreviousEvent(request.from_event_id.clone()))?;
        let artifact =
            recording
                .linux_process
                .as_ref()
                .ok_or_else(|| ReplayError::UnsupportedCapability {
                    capability: "rrTrace".into(),
                    reason: "recording does not reference an rr trace".into(),
                })?;
        if artifact.trace_directory.trim().is_empty() {
            return Err(ReplayError::UnsupportedCapability {
                capability: "rrTrace".into(),
                reason: "rr trace directory is empty".into(),
            });
        }
        let native_event = artifact
            .event_index
            .get(&event.id)
            .copied()
            .filter(|value| *value > 0)
            .ok_or_else(|| ReplayError::UnsupportedCapability {
                capability: "crossIndex".into(),
                reason: format!("Walaru event `{}` has no rr event mapping", event.id),
            })?;
        Ok(LinuxReplayPlan {
            executable: executable.clone(),
            argv: vec![
                "replay".into(),
                "-g".into(),
                native_event.to_string(),
                artifact.trace_directory.clone(),
            ],
            event,
            verified: false,
        })
    }
}

impl ReplayBackend for LinuxProcessReplayBackend {
    fn capabilities(&self) -> CapabilityManifest {
        let (completeness, supported, unavailable) = if self.rr_executable.is_some() {
            (
                Completeness::Partial,
                vec![
                    "rrAdapter".into(),
                    "crossIndex".into(),
                    "navigationPlan".into(),
                ],
                [
                    (
                        "exactStateProof".into(),
                        "native/JVM observable state proof is not captured".into(),
                    ),
                    (
                        "checkpoint".into(),
                        "CRIU/CRaC adapter not configured".into(),
                    ),
                ]
                .into_iter()
                .collect(),
            )
        } else {
            (
                Completeness::Unsupported,
                vec!["crossIndex".into()],
                [("rr".into(), "rr executable was not detected".into())]
                    .into_iter()
                    .collect(),
            )
        };
        CapabilityManifest {
            backend: "linux-process".into(),
            completeness,
            supported,
            unavailable,
        }
    }

    fn reverse(
        &self,
        recording: &Recording,
        request: &ReverseRequest,
    ) -> Result<ReverseOutcome, ReplayError> {
        let _plan = self.plan_reverse(recording, request)?;
        Err(ReplayError::UnsupportedCapability {
            capability: "exactStateProof".into(),
            reason: "rr navigation is available, but the v1 native/JVM state contract cannot yet be verified"
                .into(),
        })
    }
}

fn validate_recording(recording: &Recording) -> Result<(), ReplayError> {
    let mut previous_sequence = None;
    for checkpoint in &recording.checkpoints {
        if previous_sequence.is_some_and(|previous| previous >= checkpoint.sequence) {
            return Err(ReplayError::Diverged {
                sequence: checkpoint.sequence,
                reason: "recording checkpoints are not strictly ordered".into(),
            });
        }
        let Some(event) = recording
            .events
            .iter()
            .find(|event| event.sequence == checkpoint.sequence && event.id == checkpoint.event_id)
        else {
            return Err(ReplayError::Diverged {
                sequence: checkpoint.sequence,
                reason: "recording checkpoint does not reference an event".into(),
            });
        };
        if event.state_hash != checkpoint.state_hash
            || event.output_index != checkpoint.output_index
        {
            return Err(ReplayError::Diverged {
                sequence: checkpoint.sequence,
                reason: "recording checkpoint state is inconsistent".into(),
            });
        }
        previous_sequence = Some(checkpoint.sequence);
    }
    if recording.capabilities.completeness != Completeness::Complete {
        let (capability, reason) = recording
            .capabilities
            .unavailable
            .iter()
            .next()
            .map_or_else(
                || ("completeness".into(), "recording is marked partial".into()),
                |(capability, reason)| (capability.clone(), reason.clone()),
            );
        return Err(ReplayError::UnsupportedCapability { capability, reason });
    }
    for required in ["pureJvm"] {
        if !recording
            .capabilities
            .supported
            .iter()
            .any(|value| value == required)
        {
            return Err(ReplayError::UnsupportedCapability {
                capability: required.into(),
                reason: format!("recording does not declare `{required}`"),
            });
        }
    }
    let single_thread = recording
        .capabilities
        .supported
        .iter()
        .any(|value| value == "singleThread");
    let scheduled_threads = recording
        .capabilities
        .supported
        .iter()
        .any(|value| value == "threadSchedule");
    if !single_thread && !scheduled_threads {
        return Err(ReplayError::UnsupportedCapability {
            capability: "threads".into(),
            reason: "recording declares neither single-thread execution nor a replay schedule"
                .into(),
        });
    }
    Ok(())
}

fn matches_request(event: &Event, request: &ReverseRequest) -> bool {
    if let Some(step) = request.step {
        let step_matches = matches!(
            (step, event.kind),
            (StepKind::Line, EventKind::Line)
                | (StepKind::Call, EventKind::Call)
                | (StepKind::Write, EventKind::Write)
        );
        return step_matches
            && request
                .watch
                .as_deref()
                .is_none_or(|watch| matches_watchpoint(&event.values, watch));
    }
    request.until.as_ref().is_some_and(|target| {
        event.location.as_ref().is_some_and(|location| {
            normalize_path(&location.path) == normalize_path(&target.path)
                && location.line == target.line
        })
    })
}

fn valid_watchpoint(watch: &str) -> bool {
    watch
        .split_once('#')
        .is_some_and(|(owner, field)| !owner.trim().is_empty() && !field.trim().is_empty())
        || watch
            .strip_prefix("array[")
            .and_then(|value| value.strip_suffix(']'))
            .is_some_and(|index| index.parse::<i64>().is_ok())
}

fn matches_watchpoint(values: &Value, watch: &str) -> bool {
    if let Some(index) = watch
        .strip_prefix("array[")
        .and_then(|value| value.strip_suffix(']'))
        .and_then(|value| value.parse::<i64>().ok())
    {
        return values.get("targetKind").and_then(Value::as_str) == Some("array")
            && values.get("index").and_then(Value::as_i64) == Some(index);
    }
    let Some((owner, field)) = watch.split_once('#') else {
        return false;
    };
    let expected_owner = owner.trim().replace('.', "/");
    values.get("targetKind").and_then(Value::as_str) == Some("field")
        && values
            .get("targetOwner")
            .and_then(Value::as_str)
            .is_some_and(|actual| actual.replace('.', "/") == expected_owner)
        && values.get("field").and_then(Value::as_str) == Some(field.trim())
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
}
