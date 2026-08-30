//! Concurrent cold-start contract shared by editor and CLI clients.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

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
    let mut cleanup = DaemonCleanup::new(root.clone());
    let barrier = Arc::new(Barrier::new(CLIENTS));
    let clients = (0..CLIENTS)
        .map(|index| {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut command = Command::new(WALARU);
                command.args([
                    "--workspace",
                    root.to_str().unwrap(),
                    "--format",
                    "json",
                    if index % 2 == 0 { "status" } else { "tests" },
                ]);
                bounded_output(command, Duration::from_secs(10))
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

    let stopped = cleanup.stop();
    assert_eq!(stopped.status.code(), Some(0));
}

struct DaemonCleanup {
    root: PathBuf,
    armed: bool,
}

impl DaemonCleanup {
    fn new(root: PathBuf) -> Self {
        Self { root, armed: true }
    }

    fn stop(&mut self) -> std::process::Output {
        let output = stop_daemon(&self.root);
        if output.status.success() {
            self.armed = false;
        }
        output
    }
}

impl Drop for DaemonCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = stop_daemon(&self.root);
        }
    }
}

fn stop_daemon(root: &std::path::Path) -> Output {
    let mut command = Command::new(WALARU);
    command.args([
        "--workspace",
        root.to_str().unwrap(),
        "--format",
        "json",
        "stop",
    ]);
    bounded_output(command, Duration::from_secs(5))
}

fn bounded_output(mut command: Command, timeout: Duration) -> Output {
    // Capture into regular files instead of pipes. A daemonized grandchild retaining a Windows
    // pipe handle would otherwise make `wait_with_output` wait for EOF after the client exited.
    let mut stdout = tempfile::tempfile().unwrap();
    let mut stderr = tempfile::tempfile().unwrap();
    command
        .stdout(Stdio::from(stdout.try_clone().unwrap()))
        .stderr(Stdio::from(stderr.try_clone().unwrap()));
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Output {
                status,
                stdout: read_capture(&mut stdout),
                stderr: read_capture(&mut stderr),
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let kill_deadline = Instant::now() + Duration::from_secs(2);
            while child.try_wait().unwrap().is_none() && Instant::now() < kill_deadline {
                thread::sleep(Duration::from_millis(20));
            }
            let stdout = read_capture(&mut stdout);
            let stderr = read_capture(&mut stderr);
            panic!(
                "command timed out after {timeout:?}\nstderr: {}\nstdout: {}",
                String::from_utf8_lossy(&stderr),
                String::from_utf8_lossy(&stdout)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn read_capture(file: &mut fs::File) -> Vec<u8> {
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}
