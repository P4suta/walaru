//! Worktree-local daemon and framed protobuf transport.

mod overlay;
mod server;
mod verifier;

pub use overlay::{
    MAX_OVERLAY_BYTES, OverlayDocument, OverlayError, OverlayManifest, OverlayRequest,
};
pub use server::{Daemon, DaemonError, DaemonServer, daemon_is_running, send_request};
pub use verifier::{
    CancellationToken, LiveValueHint, RecordingOptions, RuntimeArtifacts, VerificationMode,
    VerificationOutcome, VerificationRequest, Verifier, VerifierError, WorkerProblem,
};
