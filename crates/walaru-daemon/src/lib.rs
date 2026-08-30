//! Worktree-local daemon and framed protobuf transport.

mod server;
mod verifier;

pub use server::{Daemon, DaemonError, DaemonServer, daemon_is_running, send_request};
pub use verifier::{
    RecordingOptions, RuntimeArtifacts, VerificationMode, VerificationOutcome, VerificationRequest,
    Verifier, VerifierError,
};
