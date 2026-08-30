//! Stable, content-bound identifiers shared by the CLI, daemon, and JVM worker.

use std::fmt::{self, Display};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A stable identifier for a canonical worktree path.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    /// Derives an ID after normalizing separators and trailing slashes.
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        let normalized = canonical_path(path);
        Self(format!("ws-{}", short_sha256(normalized.as_bytes(), 8)))
    }

    /// Returns the serialized identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for WorkspaceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// SHA-256 identity of all execution-relevant workspace inputs.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RevisionId(String);

impl RevisionId {
    /// Wraps an already calculated 256-bit content digest.
    #[must_use]
    pub fn from_digest(digest: [u8; 32]) -> Self {
        Self(format!("rev-{}", hex::encode(digest)))
    }

    /// Hashes a canonical stream of revision inputs.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_digest(Sha256::digest(bytes).into())
    }

    /// Returns the serialized identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for RevisionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Canonical fields that make an event unique inside a revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventIdentity {
    /// Revision observed by the worker.
    pub revision: RevisionId,
    /// Owning run.
    pub run_id: String,
    /// Stable public test identifier.
    pub test_id: String,
    /// Monotonic sequence within the run.
    pub sequence: u64,
    /// JVM thread identifier.
    pub thread_id: u64,
}

/// Stable event ID. It is deterministic but intentionally not used as ordering.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    /// Hashes the versioned canonical event identity.
    #[must_use]
    pub fn new(identity: &EventIdentity) -> Self {
        let mut canonical = Vec::new();
        for part in [
            "event-v1",
            identity.revision.as_str(),
            &identity.run_id,
            &identity.test_id,
            &identity.sequence.to_string(),
            &identity.thread_id.to_string(),
        ] {
            canonical.extend_from_slice(part.as_bytes());
            canonical.push(0);
        }
        Self(format!("evt-{}", short_sha256(&canonical, 12)))
    }

    /// Returns the serialized identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn canonical_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let mut components = Vec::new();
    for component in replaced.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    format!("/{}", components.join("/"))
}

fn short_sha256(value: &[u8], bytes: usize) -> String {
    let digest = Sha256::digest(value);
    hex::encode(&digest[..bytes])
}
