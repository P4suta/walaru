//! Public CLI, format, daemon autostart, and exit-code contract.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::tempdir;
use walaru_core::workspace::WorkspaceLayout;

const WALARU: &str = env!("CARGO_BIN_EXE_walaru");

#[test]
fn help_exposes_the_frozen_command_surface() {
    let output = Command::new(WALARU).arg("--help").output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for command in [
        "status", "watch", "tui", "stop", "cancel", "doctor", "verify", "explain", "tests",
        "failure", "impact", "coverage", "trace", "values", "record", "replay", "reverse",
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

    let restarted = Command::new(WALARU)
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "--format",
            "json",
            "status",
        ])
        .output()
        .unwrap();
    assert_eq!(
        restarted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&restarted.stderr)
    );
    let _ = Command::new(WALARU)
        .args(["--workspace", directory.path().to_str().unwrap(), "stop"])
        .output();
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
fn cancel_interrupts_an_active_worker_and_returns_a_structured_superseded_result() {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/java/demo/App.java"),
        "package demo; final class App {}\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"cancel\"",
    )
    .unwrap();
    let wrapper = directory.path().join("gradlew");
    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nset -eu\nmkdir -p .gradle\ntouch .gradle/worker-started\nsleep 30\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    for artifact in ["adapter.jar", "agent.jar", "init.gradle.kts"] {
        fs::write(directory.path().join(artifact), "fixture").unwrap();
    }
    let configure = |process: &mut Command| {
        process
            .env("WALARU_ADAPTER_JAR", directory.path().join("adapter.jar"))
            .env("WALARU_AGENT_JAR", directory.path().join("agent.jar"))
            .env(
                "WALARU_INIT_SCRIPT",
                directory.path().join("init.gradle.kts"),
            );
    };
    let mut verify = Command::new(WALARU);
    verify
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "--format",
            "json",
            "verify",
            "--supersede",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure(&mut verify);
    let verify = verify.spawn().unwrap();
    let marker = directory.path().join(".gradle/worker-started");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(marker.is_file(), "verification worker did not start");

    let started = Instant::now();
    let mut cancel = Command::new(WALARU);
    cancel.args([
        "--workspace",
        directory.path().to_str().unwrap(),
        "--format",
        "json",
        "cancel",
    ]);
    configure(&mut cancel);
    let cancelled = cancel.output().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "cancel took {:?}",
        started.elapsed()
    );
    assert_eq!(cancelled.status.code(), Some(0));
    let envelope: Value = serde_json::from_slice(&cancelled.stdout).unwrap();
    assert_eq!(envelope["data"]["cancelled"], true);

    let verified = verify.wait_with_output().unwrap();
    assert_eq!(
        verified.status.code(),
        Some(4),
        "stderr: {}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let envelope: Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(envelope["status"], "stale");
    assert_eq!(envelope["data"]["cancelled"], true);

    let _ = Command::new(WALARU)
        .args(["--workspace", directory.path().to_str().unwrap(), "stop"])
        .output();
}

#[test]
fn verify_executes_unsaved_overlay_without_modifying_the_workspace() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("src/main/java/demo/App.java");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(
        &source,
        "package demo; final class App { int value = 1; }\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"overlay\"",
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
cat src/main/java/demo/App.java > "$event_file.source"
printf '%s\n' "$@" > "$event_file.args"
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"type":"TEST_START","testName":"demo.AppTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"type":"TEST_FINISH","testName":"demo.AppTest#works","status":"successful","stateHash":"s1"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    for artifact in ["adapter.jar", "agent.jar", "init.gradle.kts"] {
        fs::write(directory.path().join(artifact), "fixture").unwrap();
    }
    let manifest = directory.path().join("overlay.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "sessionId": "cli-test",
            "documents": [{
                "path": "src/main/java/demo/App.java",
                "version": 42,
                "content": "package demo; final class App { int value = 2; }\n"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut verify = Command::new(WALARU);
    verify.args([
        "--workspace",
        directory.path().to_str().unwrap(),
        "--format",
        "json",
        "verify",
        "--overlay-manifest",
        manifest.to_str().unwrap(),
        "--test",
        "demo.AppTest#works",
        "--supersede",
    ]);
    for (name, file) in [
        ("WALARU_ADAPTER_JAR", "adapter.jar"),
        ("WALARU_AGENT_JAR", "agent.jar"),
        ("WALARU_INIT_SCRIPT", "init.gradle.kts"),
    ] {
        verify.env(name, directory.path().join(file));
    }

    let verified = verify.output().unwrap();

    assert_eq!(
        verified.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let envelope: Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(
        envelope["data"]["overlayVersions"]["src/main/java/demo/App.java"],
        42
    );
    assert_eq!(envelope["data"]["selection"], "explicit");
    let worker_directory = WorkspaceLayout::new(directory.path())
        .unwrap()
        .state_dir
        .join("live/cli-test/worker");
    assert!(
        fs::read_to_string(worker_directory.join("events.jsonl.source"))
            .unwrap()
            .contains("value = 2")
    );
    assert!(
        fs::read_to_string(worker_directory.join("events.jsonl.args"))
            .unwrap()
            .contains("-Dwalaru.tests=demo.AppTest.works")
    );
    assert!(fs::read_to_string(&source).unwrap().contains("value = 1"));

    let _ = Command::new(WALARU)
        .args(["--workspace", directory.path().to_str().unwrap(), "stop"])
        .output();
}

#[test]
fn invalid_overlay_is_usage_error_and_does_not_start_a_daemon() {
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"invalid-overlay\"",
    )
    .unwrap();
    let manifest = directory.path().join("overlay.json");
    fs::write(
        &manifest,
        r#"{"schemaVersion":2,"sessionId":"vscode","documents":[]}"#,
    )
    .unwrap();
    let layout = WorkspaceLayout::new(directory.path()).unwrap();

    let result = Command::new(WALARU)
        .args([
            "--workspace",
            directory.path().to_str().unwrap(),
            "verify",
            "--overlay-manifest",
            manifest.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("invalid overlay manifest"));
    assert!(!layout.state_dir.exists());
}

#[test]
fn explain_verifies_diagnoses_redacts_and_records_a_failure_in_one_command() {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
    fs::write(
        directory
            .path()
            .join("src/main/java/demo/BinarySearch.java"),
        "package demo; public final class BinarySearch {}\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"cli-explain\"",
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
'{"schemaVersion":1,"sequence":0,"threadId":1,"type":"TEST_START","testId":"id","testName":"demo.BinarySearchTest#findsLast","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"type":"CAPTURE","testId":"id","testName":"demo.BinarySearchTest#findsLast","owner":"demo/BinarySearch","method":"find","path":"src/main/java/demo/BinarySearch.java","line":12,"values":{"name":"target","value":9,"redacted":false},"stateHash":"s1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"type":"CHECKPOINT","testId":"id","testName":"demo.BinarySearchTest#findsLast","owner":"demo/BinarySearch","method":"find","path":"src/main/java/demo/BinarySearch.java","line":18,"values":{"name":"partition","value":{"low":4,"high":4}},"stateHash":"s2"}' \
'{"schemaVersion":1,"sequence":3,"threadId":1,"type":"CAPTURE","testId":"id","testName":"demo.BinarySearchTest#findsLast","owner":"demo/BinarySearch","method":"find","path":"src/main/java/demo/BinarySearch.java","line":21,"values":{"name":"apiToken","value":"must-not-leak","redacted":true},"stateHash":"s3"}' \
'{"schemaVersion":1,"sequence":4,"threadId":1,"type":"TEST_FINISH","testId":"id","testName":"demo.BinarySearchTest#findsLast","status":"failed","failureType":"org.opentest4j.AssertionFailedError","message":"expected: <4> but was: <-1>","frames":["demo.BinarySearchTest.findsLast(BinarySearchTest.java:9)"],"stateHash":"s4"}' > "$event_file"
exit 1
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    for artifact in ["adapter.jar", "agent.jar", "init.gradle.kts"] {
        fs::write(directory.path().join(artifact), "fixture").unwrap();
    }
    let run = |command: &[&str]| {
        let mut process = Command::new(WALARU);
        process.args(["--workspace", directory.path().to_str().unwrap()]);
        process.args(command);
        for (name, file) in [
            ("WALARU_ADAPTER_JAR", "adapter.jar"),
            ("WALARU_AGENT_JAR", "agent.jar"),
            ("WALARU_INIT_SCRIPT", "init.gradle.kts"),
        ] {
            process.env(name, directory.path().join(file));
        }
        process.output().unwrap()
    };

    let explained = run(&["explain", "--max-failures", "1"]);
    assert_eq!(
        explained.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&explained.stderr)
    );
    let envelope: Value = serde_json::from_slice(&explained.stdout).unwrap();
    assert_eq!(envelope["status"], "failure");
    assert_eq!(envelope["data"]["verification"]["status"], "failed");
    assert_eq!(
        envelope["data"]["explanations"][0]["failure"]["testId"],
        "demo.BinarySearchTest#findsLast"
    );
    assert_eq!(
        envelope["data"]["explanations"][0]["analysis"]["focus"]["line"],
        18
    );
    assert!(
        envelope["data"]["explanations"][0]["analysis"]["summary"]
            .as_str()
            .unwrap()
            .contains("expected 4, observed -1")
    );
    assert!(
        envelope["data"]["explanations"][0]["recording"]["id"]
            .as_str()
            .unwrap()
            .starts_with("rec-")
    );
    let rendered = String::from_utf8(explained.stdout).unwrap();
    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("must-not-leak"));

    let human = run(&["--format", "human", "explain", "--max-failures", "1"]);
    assert_eq!(human.status.code(), Some(1));
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("Assertion failed: expected 4, observed -1."));
    assert!(human.contains("Full recording: rec-"));
    assert!(human.contains("src/main/java/demo/BinarySearch.java:18"));

    fs::write(
        &wrapper,
        "#!/usr/bin/env bash\nprintf 'error: cannot resolve symbol\\n' >&2\nexit 1\n",
    )
    .unwrap();
    let build_failure = run(&["explain", "--max-failures", "1"]);
    assert_eq!(build_failure.status.code(), Some(1));
    let envelope: Value = serde_json::from_slice(&build_failure.stdout).unwrap();
    assert!(
        envelope["data"]["explanations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        envelope["data"]["buildFailure"]["summary"]
            .as_str()
            .unwrap()
            .contains("before a structured test failure")
    );
    assert!(
        envelope["data"]["buildFailure"]["logFile"]
            .as_str()
            .unwrap()
            .ends_with("worker.log")
    );

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
