//! Public CLI, format, daemon autostart, and exit-code contract.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

const WALARU: &str = env!("CARGO_BIN_EXE_walaru");

#[test]
fn help_exposes_the_frozen_command_surface() {
    let output = Command::new(WALARU).arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for command in [
        "status", "watch", "tui", "stop", "doctor", "verify", "tests", "failure", "impact",
        "coverage", "trace", "values", "record", "replay", "reverse",
    ] {
        assert!(help.contains(command), "missing `{command}` in:\n{help}");
    }
    let record = Command::new(WALARU)
        .args(["record", "--help"])
        .output()
        .unwrap();
    assert!(record.status.success());
    assert!(String::from_utf8_lossy(&record.stdout).contains("--capture-file-io"));
}

#[test]
fn tui_once_uses_the_same_daemon_query_api_and_returns_a_bounded_dashboard() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name = \"tui-fixture\"\n",
    )
    .unwrap();

    let output = Command::new(WALARU)
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "tui",
            "--once",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dashboard = String::from_utf8(output.stdout).unwrap();
    assert!(dashboard.contains("Walaru dashboard"));
    assert!(dashboard.contains("daemon: running"));
    assert!(dashboard.len() < 8 * 1024);

    let _ = Command::new(WALARU)
        .args(["--workspace", directory.path().to_str().unwrap(), "stop"])
        .output();
}

#[test]
fn interactive_tui_rejects_non_tty_input_with_usage_exit() {
    let directory = tempdir().unwrap();
    let output = Command::new(WALARU)
        .args(["--workspace", directory.path().to_str().unwrap(), "tui"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a TTY"));
}

#[test]
fn non_tty_status_autostarts_daemon_and_defaults_to_json() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name = \"cli-fixture\"\n",
    )
    .unwrap();

    let status = Command::new(WALARU)
        .args(["--workspace", directory.path().to_str().unwrap(), "status"])
        .output()
        .unwrap();
    assert_eq!(
        status.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let envelope: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(envelope["schemaVersion"], "1");
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["data"]["running"], true);
    assert!(envelope["workspaceId"].as_str().unwrap().starts_with("ws-"));

    let stop = Command::new(WALARU)
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "--format",
            "json",
            "stop",
        ])
        .output()
        .unwrap();
    assert_eq!(stop.status.code(), Some(0));
    let stopped: Value = serde_json::from_slice(&stop.stdout).unwrap();
    assert_eq!(stopped["data"]["stopping"], true);
}

#[test]
fn stale_endpoint_and_wal_store_recover_after_an_unclean_daemon_exit() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name = \"crash-recovery\"\n",
    )
    .unwrap();
    let run = |command: &str| {
        Command::new(WALARU)
            .args([
                "--workspace",
                directory.path().to_str().unwrap(),
                "--format",
                "json",
                command,
            ])
            .output()
            .unwrap()
    };

    let first = run("status");
    assert_eq!(first.status.code(), Some(0));
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();
    let first_pid = first["data"]["pid"].as_u64().unwrap().to_string();
    assert!(
        Command::new("kill")
            .args(["-9", &first_pid])
            .status()
            .unwrap()
            .success()
    );
    std::thread::sleep(std::time::Duration::from_millis(100));

    let recovered = run("status");
    assert_eq!(
        recovered.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    let recovered: Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert_ne!(
        recovered["data"]["pid"].as_u64().unwrap().to_string(),
        first_pid
    );
    assert_eq!(recovered["data"]["running"], true);

    assert_eq!(run("stop").status.code(), Some(0));
}

#[test]
fn reverse_requires_exactly_one_step_or_until() {
    let directory = tempdir().unwrap();
    let missing = Command::new(WALARU)
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "reverse",
            "rec-1",
            "--from",
            "evt-2",
        ])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));

    let conflicting = Command::new(WALARU)
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "reverse",
            "rec-1",
            "--from",
            "evt-2",
            "--step",
            "line",
            "--until",
            "src/main/kotlin/demo/Example.kt:10",
        ])
        .output()
        .unwrap();
    assert_eq!(conflicting.status.code(), Some(2));

    let help = Command::new(WALARU)
        .args(["reverse", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--watch"));

    let invalid_watch = Command::new(WALARU)
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "reverse",
            "rec-1",
            "--from",
            "evt-2",
            "--step",
            "line",
            "--watch",
            "demo.Counter#value",
        ])
        .output()
        .unwrap();
    assert_eq!(invalid_watch.status.code(), Some(2));
}

#[test]
fn verify_ingests_worker_events_and_queries_them_through_the_cli() {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/kotlin/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/kotlin/demo/Example.kt"),
        "package demo\nfun answer() = 1\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"cli-verify\"",
    )
    .unwrap();
    let wrapper = directory.path().join("gradlew");
    fs::write(
        &wrapper,
        r#"#!/usr/bin/env bash
set -eu
event_file=""
for argument in "$@"; do
  case "$argument" in -Dwalaru.eventFile=*) event_file="${argument#*=}" ;; esac
done
mkdir -p "$(dirname "$event_file")"
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"type":"TEST_START","testId":"id","testName":"demo.ExampleTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"type":"LINE","testId":"id","testName":"demo.ExampleTest#works","owner":"demo/ExampleKt","method":"answer","path":"Example.kt","line":2,"values":{"answer":1},"stateHash":"s1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"type":"TEST_FINISH","testId":"id","testName":"demo.ExampleTest#works","status":"successful","stateHash":"s2"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    for artifact in ["adapter.jar", "agent.jar", "init.gradle.kts"] {
        fs::write(directory.path().join(artifact), "fixture").unwrap();
    }
    let environment = [
        ("WALARU_ADAPTER_JAR", directory.path().join("adapter.jar")),
        ("WALARU_AGENT_JAR", directory.path().join("agent.jar")),
        (
            "WALARU_INIT_SCRIPT",
            directory.path().join("init.gradle.kts"),
        ),
    ];
    let run = |command: &[&str]| {
        let mut process = Command::new(WALARU);
        process.args(["--workspace", directory.path().to_str().unwrap()]);
        process.args(command);
        for (name, value) in &environment {
            process.env(name, value);
        }
        process.output().unwrap()
    };

    let verified = run(&["verify"]);
    assert_eq!(
        verified.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let envelope: Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(envelope["data"]["tests"][0], "demo.ExampleTest#works");
    assert_eq!(envelope["data"]["events"], 3);

    let tests = run(&["tests"]);
    let envelope: Value = serde_json::from_slice(&tests.stdout).unwrap();
    assert_eq!(envelope["data"]["tests"][0]["lastStatus"], "passed");
    let trace = run(&["trace", "demo.ExampleTest#works"]);
    let envelope: Value = serde_json::from_slice(&trace.stdout).unwrap();
    assert_eq!(envelope["data"]["events"][1]["values"]["answer"], 1);

    let recorded = run(&["record", "demo.ExampleTest#works"]);
    assert_eq!(recorded.status.code(), Some(0));
    let envelope: Value = serde_json::from_slice(&recorded.stdout).unwrap();
    let recording_id = envelope["data"]["recordingId"].as_str().unwrap().to_owned();
    let trace = run(&["trace", "demo.ExampleTest#works"]);
    let envelope: Value = serde_json::from_slice(&trace.stdout).unwrap();
    let from = envelope["data"]["events"][2]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reversed = run(&["reverse", &recording_id, "--from", &from, "--step", "line"]);
    assert_eq!(
        reversed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&reversed.stderr)
    );
    let envelope: Value = serde_json::from_slice(&reversed.stdout).unwrap();
    assert_eq!(envelope["data"]["verified"], true);
    assert_eq!(envelope["data"]["event"]["location"]["line"], 2);

    let stopped = run(&["stop"]);
    assert_eq!(stopped.status.code(), Some(0));
}

#[test]
fn stale_verification_is_requeued_once_for_the_latest_revision() {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/kotlin/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/kotlin/demo/Example.kt"),
        "package demo\nfun answer() = 1\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"stale-requeue\"",
    )
    .unwrap();
    let wrapper = directory.path().join("gradlew");
    fs::write(
        &wrapper,
        r#"#!/usr/bin/env bash
set -eu
event_file=""
for argument in "$@"; do
  case "$argument" in -Dwalaru.eventFile=*) event_file="${argument#*=}" ;; esac
done
mkdir -p "$(dirname "$event_file")" .gradle
printf x >> .gradle/invocations
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"type":"TEST_START","testId":"id","testName":"demo.ExampleTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"type":"TEST_FINISH","testId":"id","testName":"demo.ExampleTest#works","status":"successful","stateHash":"s1"}' > "$event_file"
if [ ! -e .gradle/edited ]; then
  printf 'package demo\nfun answer() = 2\n' > src/main/kotlin/demo/Example.kt
  touch .gradle/edited
fi
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    for artifact in ["adapter.jar", "agent.jar", "init.gradle.kts"] {
        fs::write(directory.path().join(artifact), "fixture").unwrap();
    }
    let mut process = Command::new(WALARU);
    process.args(["--workspace", directory.path().to_str().unwrap(), "verify"]);
    for (name, file) in [
        ("WALARU_ADAPTER_JAR", "adapter.jar"),
        ("WALARU_AGENT_JAR", "agent.jar"),
        ("WALARU_INIT_SCRIPT", "init.gradle.kts"),
    ] {
        process.env(name, directory.path().join(file));
    }
    let verified = process.output().unwrap();

    assert_eq!(
        verified.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let envelope: Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(envelope["status"], "ok");
    assert_eq!(envelope["data"]["status"], "passed");
    assert_eq!(
        fs::read_to_string(directory.path().join(".gradle/invocations")).unwrap(),
        "xx"
    );

    let _ = Command::new(WALARU)
        .args(["--workspace", directory.path().to_str().unwrap(), "stop"])
        .output();
}
