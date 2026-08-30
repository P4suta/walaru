//! Version 1 JSON envelope and protobuf transport messages.

use std::collections::BTreeMap;

use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current public JSON and RPC schema version.
pub const SCHEMA_VERSION: &str = "1";

/// Fixed envelope returned by every CLI query and verification operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Envelope {
    /// JSON schema version.
    pub schema_version: String,
    /// Worktree identity.
    pub workspace_id: String,
    /// Content-bound revision.
    pub revision: String,
    /// Client/daemon interaction session.
    pub session_id: String,
    /// Run identity when the response belongs to a run.
    pub run_id: Option<String>,
    /// Completion classification.
    pub status: Status,
    /// Command-specific response body.
    pub data: Value,
    /// Structured, machine-actionable diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// Honest replay and recording guarantees.
    pub capabilities: CapabilityManifest,
    /// Suggested argv vectors; never shell snippets.
    pub next_actions: Vec<NextAction>,
    /// Pagination state for bounded queries.
    pub page: Option<Page>,
}

/// Response status independent of the process exit code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// Requested work completed.
    Ok,
    /// Compilation or tests failed.
    Failure,
    /// Useful data exists but recording is incomplete.
    Partial,
    /// Workspace changed during execution.
    Stale,
    /// Required capability is unavailable.
    Unsupported,
    /// Daemon, worker, or internal failure.
    Error,
}

/// Structured diagnostic entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Diagnostic {
    /// Stable diagnostic code.
    pub code: String,
    /// `info`, `warning`, or `error`.
    pub severity: String,
    /// Human-readable summary.
    pub message: String,
    /// Bounded structured context.
    pub details: BTreeMap<String, String>,
}

/// Degree to which a recording can be replayed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Completeness {
    /// Every required boundary in the declared scope was captured.
    Complete,
    /// Some events are queryable but exact replay is not guaranteed.
    Partial,
    /// The backend cannot service this recording.
    Unsupported,
}

/// Capability contract attached to every recording and response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityManifest {
    /// Backend name (`jvm` or `linux-process`).
    pub backend: String,
    /// Overall guarantee for the requested operation.
    pub completeness: Completeness,
    /// Captured capabilities.
    pub supported: Vec<String>,
    /// Capability to precise reason mapping.
    pub unavailable: BTreeMap<String, String>,
}

impl CapabilityManifest {
    /// Empty manifest for non-recording queries.
    #[must_use]
    pub fn query_only() -> Self {
        Self {
            backend: "none".into(),
            completeness: Completeness::Unsupported,
            supported: Vec::new(),
            unavailable: BTreeMap::new(),
        }
    }
}

/// Follow-up action safe for direct process invocation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextAction {
    /// Short display label.
    pub title: String,
    /// Executable and arguments.
    pub argv: Vec<String>,
}

/// Cursor pagination metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    /// Cursor supplied by the client.
    pub cursor: Option<String>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
    /// Requested item limit.
    pub limit: usize,
    /// Items returned on this page.
    pub returned: usize,
}

/// Versioned command request carried over a worktree-local transport.
#[derive(Clone, PartialEq, Message)]
pub struct RpcRequest {
    /// Wire schema version.
    #[prost(uint32, tag = "1")]
    pub schema_version: u32,
    /// Client-generated request correlation ID.
    #[prost(string, tag = "2")]
    pub request_id: String,
    /// Absolute target worktree root.
    #[prost(string, tag = "3")]
    pub workspace_root: String,
    /// Public command name.
    #[prost(string, tag = "4")]
    pub command: String,
    /// Forward-compatible command payload.
    #[prost(bytes = "vec", tag = "5")]
    pub payload_json: Vec<u8>,
}

/// Versioned command response carried over a worktree-local transport.
#[derive(Clone, PartialEq, Message)]
pub struct RpcResponse {
    /// Wire schema version.
    #[prost(uint32, tag = "1")]
    pub schema_version: u32,
    /// Correlated request ID.
    #[prost(string, tag = "2")]
    pub request_id: String,
    /// Public CLI exit code.
    #[prost(int32, tag = "3")]
    pub exit_code: i32,
    /// Serialized [`Envelope`].
    #[prost(bytes = "vec", tag = "4")]
    pub envelope_json: Vec<u8>,
}

/// Event frame emitted by a JVM worker.
#[derive(Clone, PartialEq, Message)]
pub struct WorkerEvent {
    /// Wire schema version.
    #[prost(uint32, tag = "1")]
    pub schema_version: u32,
    /// Worktree identity.
    #[prost(string, tag = "2")]
    pub workspace_id: String,
    /// Content revision.
    #[prost(string, tag = "3")]
    pub revision: String,
    /// Owning run.
    #[prost(string, tag = "4")]
    pub run_id: String,
    /// Public test ID.
    #[prost(string, tag = "5")]
    pub test_id: String,
    /// Monotonic event sequence.
    #[prost(uint64, tag = "6")]
    pub sequence: u64,
    /// JVM thread ID.
    #[prost(uint64, tag = "7")]
    pub thread_id: u64,
    /// Event kind.
    #[prost(string, tag = "8")]
    pub kind: String,
    /// Full forward-compatible event JSON.
    #[prost(bytes = "vec", tag = "9")]
    pub event_json: Vec<u8>,
}
