//! Concurrent cold-start contract shared by editor and CLI clients.

use std::collections::BTreeSet;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;

use serde_json::Value;
use tempfile::tempdir;

const WALARU: &str = env!("CARGO_BIN_EXE_walaru");

#[test]
fn concurrent_clients_share_one_successful_daemon_cold_start() {
    const CLIENTS: usize = 8;

    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name = \"concurrent-startup\"\n",
    )
    .unwrap();

    let root = directory.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(CLIENTS));
    let clients = (0..CLIENTS)
        .map(|index| {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                Command::new(WALARU)
                    .args([
                        "--workspace",
                        root.to_str().unwrap(),
                        "--format",
                        "json",
                        if index % 2 == 0 { "status" } else { "tests" },
                    ])
                    .output()
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();

    let mut session_ids = BTreeSet::new();
    for client in clients {
        let output = client.join().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}\nstdout: {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["schemaVersion"], "1");
        assert_eq!(envelope["status"], "ok");
        session_ids.insert(envelope["sessionId"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        session_ids.len(),
        1,
        "concurrent clients must all observe the same daemon session"
    );

    let stopped = Command::new(WALARU)
        .args([
            "--workspace",
            root.to_str().unwrap(),
            "--format",
            "json",
            "stop",
        ])
        .output()
        .unwrap();
    assert_eq!(stopped.status.code(), Some(0));
}
