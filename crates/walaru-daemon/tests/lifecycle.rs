//! Daemon lifecycle and transport contract.

use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::tempdir;
use walaru_core::protocol::{Envelope, RpcRequest};
use walaru_core::workspace::WorkspaceLayout;
use walaru_daemon::{Daemon, DaemonServer, daemon_is_running, send_request};

#[test]
fn status_returns_the_fixed_envelope_and_worktree_paths() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"fixture\"",
    )
    .unwrap();
    let daemon = Daemon::open(directory.path()).unwrap();

    let response = daemon.handle(request(directory.path(), "status", 1));
    assert_eq!(response.exit_code, 0);
    let envelope: Envelope = serde_json::from_slice(&response.envelope_json).unwrap();
    assert_eq!(envelope.schema_version, "1");
    assert_eq!(envelope.status, walaru_core::protocol::Status::Ok);
    assert_eq!(envelope.data["running"], true);
    assert!(
        envelope.data["stateDirectory"]
            .as_str()
            .unwrap()
            .contains(".gradle/walaru/ws-")
    );
    assert_eq!(envelope.revision.len(), "rev-".len() + 64);
}

#[test]
fn unsupported_wire_schema_is_a_usage_error() {
    let directory = tempdir().unwrap();
    let daemon = Daemon::open(directory.path()).unwrap();
    let response = daemon.handle(request(directory.path(), "status", 99));
    assert_eq!(response.exit_code, 2);
    let envelope: Envelope = serde_json::from_slice(&response.envelope_json).unwrap();
    assert_eq!(envelope.status, walaru_core::protocol::Status::Error);
    assert_eq!(envelope.diagnostics[0].code, "WALARU_SCHEMA_VERSION");
}

#[test]
fn doctor_recognizes_a_maven_project_without_requiring_gradle() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("pom.xml"), "<project/>").unwrap();
    fs::write(directory.path().join("mvnw"), "#!/bin/sh\nexit 0\n").unwrap();
    let daemon = Daemon::open(directory.path()).unwrap();

    let response = daemon.handle(request(directory.path(), "doctor", 1));
    assert_eq!(response.exit_code, 0);
    let envelope: Envelope = serde_json::from_slice(&response.envelope_json).unwrap();
    assert_eq!(envelope.data["buildTool"]["kind"], "maven");
    assert_eq!(envelope.data["buildTool"]["project"], true);
    assert_eq!(envelope.data["buildTool"]["wrapper"], true);
    assert!(
        envelope
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "WALARU_GRADLE_WRAPPER")
    );
    assert_eq!(envelope.data["platform"]["supported"], true);
    assert!(envelope.data["linuxReplay"]["rr"].is_boolean());
    assert!(envelope.data["linuxReplay"]["criu"].is_boolean());
    assert!(envelope.data["linuxReplay"]["crac"].is_boolean());
    assert_eq!(envelope.data["linuxReplay"]["required"], false);
}

#[test]
fn local_server_round_trips_protobuf_and_stop_removes_endpoint() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("build.gradle.kts"),
        "plugins { java }",
    )
    .unwrap();
    let workspace = directory.path().to_path_buf();
    let layout = WorkspaceLayout::new(&workspace).unwrap();
    let server_workspace = workspace.clone();
    let (exit_tx, exit_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let result = DaemonServer::serve(server_workspace);
        let _ = exit_tx.send(result.as_ref().err().map(ToString::to_string));
        result
    });
    wait_for_socket(&layout.socket, &server, &exit_rx);

    // A connect-only health probe must not terminate the daemon on any transport.
    assert!(daemon_is_running(&layout.socket));

    let status = send_request(&layout.socket, &request(&workspace, "status", 1)).unwrap();
    assert_eq!(status.exit_code, 0);
    let stop = send_request(&layout.socket, &request(&workspace, "stop", 1)).unwrap();
    assert_eq!(stop.exit_code, 0);
    server.join().unwrap().unwrap();
    assert!(!layout.socket.exists());
}

#[cfg(unix)]
#[test]
fn long_worktree_path_uses_the_short_endpoint_and_still_round_trips() {
    let directory = tempdir().unwrap();
    let workspace = directory.path().join("nested-worktree-".repeat(8));
    fs::create_dir_all(&workspace).unwrap();
    let layout = WorkspaceLayout::new(&workspace).unwrap();
    assert!(layout.socket.starts_with("/tmp"));
    let server_workspace = workspace.clone();
    let (exit_tx, exit_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let result = DaemonServer::serve(server_workspace);
        let _ = exit_tx.send(result.as_ref().err().map(ToString::to_string));
        result
    });
    wait_for_socket(&layout.socket, &server, &exit_rx);
    assert_eq!(
        send_request(&layout.socket, &request(&workspace, "status", 1))
            .unwrap()
            .exit_code,
        0
    );
    send_request(&layout.socket, &request(&workspace, "stop", 1)).unwrap();
    server.join().unwrap().unwrap();
    assert!(!layout.socket.exists());
}

#[test]
fn independent_worktrees_and_concurrent_clients_never_share_state() {
    let left = tempdir().unwrap();
    let right = tempdir().unwrap();
    let mut servers = Vec::new();
    for workspace in [left.path().to_path_buf(), right.path().to_path_buf()] {
        fs::write(
            workspace.join("settings.gradle.kts"),
            format!("rootProject.name=\"{}\"", workspace.display()),
        )
        .unwrap();
        let layout = WorkspaceLayout::new(&workspace).unwrap();
        let server_workspace = workspace.clone();
        let server = thread::spawn(move || DaemonServer::serve(server_workspace));
        let (exit_tx, exit_rx) = mpsc::channel();
        wait_for_socket(&layout.socket, &server, &exit_rx);
        drop(exit_tx);
        servers.push((workspace, layout, server));
    }
    assert_ne!(servers[0].1.workspace_id, servers[1].1.workspace_id);
    assert_ne!(servers[0].1.database, servers[1].1.database);

    let barrier = Arc::new(Barrier::new(17));
    let mut clients = Vec::new();
    for index in 0..16 {
        let workspace = servers[index % 2].0.clone();
        let socket = servers[index % 2].1.socket.clone();
        let barrier = Arc::clone(&barrier);
        clients.push(thread::spawn(move || {
            barrier.wait();
            send_request(&socket, &request(&workspace, "status", 1)).unwrap()
        }));
    }
    barrier.wait();
    for client in clients {
        assert_eq!(client.join().unwrap().exit_code, 0);
    }
    for (workspace, layout, server) in servers {
        assert_eq!(
            send_request(&layout.socket, &request(&workspace, "stop", 1))
                .unwrap()
                .exit_code,
            0
        );
        server.join().unwrap().unwrap();
    }
}

#[test]
fn warm_local_status_query_p95_is_below_one_hundred_milliseconds() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"latency\"",
    )
    .unwrap();
    let workspace = directory.path().to_path_buf();
    let layout = WorkspaceLayout::new(&workspace).unwrap();
    let server_workspace = workspace.clone();
    let (exit_tx, exit_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let result = DaemonServer::serve(server_workspace);
        let _ = exit_tx.send(result.as_ref().err().map(ToString::to_string));
        result
    });
    wait_for_socket(&layout.socket, &server, &exit_rx);
    send_request(&layout.socket, &request(&workspace, "status", 1)).unwrap();
    let mut samples = (0..60)
        .map(|_| {
            let started = Instant::now();
            let response = send_request(&layout.socket, &request(&workspace, "status", 1)).unwrap();
            assert_eq!(response.exit_code, 0);
            started.elapsed()
        })
        .collect::<Vec<_>>();
    samples.sort_unstable();
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];

    assert!(
        p95 < Duration::from_millis(100),
        "warm query p95 was {p95:?}"
    );
    send_request(&layout.socket, &request(&workspace, "stop", 1)).unwrap();
    server.join().unwrap().unwrap();
}

fn request(workspace: &std::path::Path, command: &str, schema_version: u32) -> RpcRequest {
    RpcRequest {
        schema_version,
        request_id: format!("request-{command}"),
        workspace_root: workspace.to_string_lossy().into_owned(),
        command: command.into(),
        payload_json: b"{}".to_vec(),
    }
}

fn wait_for_socket(
    path: &std::path::Path,
    server: &thread::JoinHandle<Result<(), walaru_daemon::DaemonError>>,
    exit_rx: &mpsc::Receiver<Option<String>>,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() && !server.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !server.is_finished(),
        "daemon exited before accepting requests: {}",
        exit_rx
            .try_recv()
            .ok()
            .flatten()
            .unwrap_or_else(|| "no error".into())
    );
    assert!(
        path.exists(),
        "daemon socket was not created: {}",
        path.display()
    );
}
