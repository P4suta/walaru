//! Gradle process, revision, ingest, and recording contract.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

use tempfile::tempdir;
use walaru_core::store::{RetentionPolicy, RunStatus, Store};
use walaru_core::workspace::WorkspaceLayout;
use walaru_daemon::VerifierError;
use walaru_daemon::{RecordingOptions, RuntimeArtifacts, VerificationRequest, Verifier};

#[test]
fn successful_gradle_run_ingests_tests_coverage_trace_and_values() {
    let fixture = fixture(false, false);
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let outcome = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10))
        .verify(&VerificationRequest::fast())
        .unwrap();

    assert_eq!(outcome.status, RunStatus::Passed);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.tests, vec!["demo.ExampleTest#works"]);
    assert_eq!(
        store.tests(None, 100).unwrap().items[0]
            .last_status
            .as_deref(),
        Some("passed")
    );
    let trace = store
        .trace("demo.ExampleTest#works", None, 100, 65_536)
        .unwrap();
    assert_eq!(trace.items.len(), 3);
    assert_eq!(trace.items[1].values["counter"], 1);
    assert_eq!(
        store
            .coverage("src/main/kotlin/demo/Example.kt", None, 100)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn source_edit_during_gradle_run_is_stale_even_when_tests_pass() {
    let fixture = fixture(true, false);
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let outcome = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10))
        .verify(&VerificationRequest::fast())
        .unwrap();

    assert_eq!(outcome.status, RunStatus::Stale);
    assert_eq!(outcome.exit_code, 4);
}

#[test]
fn unchanged_fast_verification_uses_revision_cache_without_starting_gradle() {
    let fixture = fixture(false, false);
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));

    let first = verifier.verify(&VerificationRequest::fast()).unwrap();
    let mut samples = Vec::new();
    let mut second = None;
    for _ in 0..20 {
        let started = Instant::now();
        second = Some(verifier.verify(&VerificationRequest::fast()).unwrap());
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    let second = second.unwrap();

    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(first.run_id, second.run_id);
    assert!(
        p95 < Duration::from_millis(250),
        "no-change verify p95 was {p95:?}"
    );
    assert_eq!(
        fs::read_to_string(fixture.path().join(".gradle/fake-invocations")).unwrap(),
        "x"
    );
}

#[test]
fn malformed_worker_trace_and_timeout_leave_no_running_store_record() {
    let malformed = fixture(false, false);
    let malformed_wrapper = malformed.path().join("gradlew");
    fs::write(
        &malformed_wrapper,
        r#"#!/usr/bin/env bash
set -eu
for argument in "$@"; do
  case "$argument" in -Dwalaru.eventFile=*) event_file="${argument#*=}" ;; esac
done
printf '{not-json}\n' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&malformed_wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let layout = WorkspaceLayout::new(malformed.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();

    let error = Verifier::new(&layout, &store, artifacts(malformed.path()))
        .with_timeout(Duration::from_secs(5))
        .verify(&VerificationRequest::fast())
        .unwrap_err();

    assert!(matches!(error, VerifierError::Json(_)));
    assert_only_run_is_error(&layout, &store);

    let timed_out = fixture(false, false);
    let timeout_wrapper = timed_out.path().join("gradlew");
    fs::write(&timeout_wrapper, "#!/usr/bin/env bash\nsleep 30\n").unwrap();
    fs::set_permissions(&timeout_wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let timeout_layout = WorkspaceLayout::new(timed_out.path()).unwrap();
    timeout_layout.ensure_state_dir().unwrap();
    let timeout_store = Store::open(&timeout_layout.database, RetentionPolicy::default()).unwrap();
    let started = Instant::now();

    let error = Verifier::new(&timeout_layout, &timeout_store, artifacts(timed_out.path()))
        .with_timeout(Duration::from_millis(150))
        .verify(&VerificationRequest::fast())
        .unwrap_err();

    assert!(matches!(error, VerifierError::Timeout(_)));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_only_run_is_error(&timeout_layout, &timeout_store);
}

#[test]
fn implementation_only_change_selects_impacted_tests_but_abi_change_expands_to_all() {
    let fixture = fixture(false, false);
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));
    verifier.verify(&VerificationRequest::fast()).unwrap();

    fs::write(
        fixture.path().join("src/main/kotlin/demo/Example.kt"),
        "package demo\nfun answer() = 2\n",
    )
    .unwrap();
    verifier.verify(&VerificationRequest::fast()).unwrap();
    let selected = fs::read_to_string(fixture.path().join(".gradle/last-args")).unwrap();
    assert!(
        selected.contains("-Dwalaru.tests=demo.ExampleTest.works"),
        "impact filter was not passed to Gradle:\n{selected}"
    );

    fs::write(
        fixture.path().join("src/main/kotlin/demo/Example.kt"),
        "package demo\nfun answer(value: Int) = value + 2\n",
    )
    .unwrap();
    verifier.verify(&VerificationRequest::fast()).unwrap();
    let expanded = fs::read_to_string(fixture.path().join(".gradle/last-args")).unwrap();
    assert!(
        !expanded.contains("-Dwalaru.tests="),
        "ABI change must expand to all tests:\n{expanded}"
    );
}

#[test]
fn multi_module_events_keep_module_identity_and_select_only_the_impacted_test() {
    let fixture = multi_module_fixture();
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));

    let first = verifier.verify(&VerificationRequest::fast()).unwrap();
    assert_eq!(
        first.tests,
        vec![
            ":alpha::demo.SharedTest#works",
            ":beta::demo.SharedTest#works",
        ]
    );
    let tests = store.tests(None, 100).unwrap().items;
    assert_eq!(
        tests
            .iter()
            .map(|test| (test.id.as_str(), test.module.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (":alpha::demo.SharedTest#works", ":alpha"),
            (":beta::demo.SharedTest#works", ":beta"),
        ]
    );
    let events = store
        .events(&first.run_id, None, 100, 65_536)
        .unwrap()
        .items;
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5]
    );

    fs::write(
        fixture
            .path()
            .join("alpha/src/main/kotlin/demo/Production.kt"),
        "package demo\nfun alphaAnswer() = 2\n",
    )
    .unwrap();
    verifier.verify(&VerificationRequest::fast()).unwrap();
    let arguments = fs::read_to_string(fixture.path().join(".gradle/last-args")).unwrap();
    assert!(
        arguments.contains("-Dwalaru.tests=:alpha::demo.SharedTest.works"),
        "multi-module impact filter was not precise:\n{arguments}"
    );
    assert!(!arguments.contains(":beta::demo.SharedTest.works"));
}

#[test]
fn since_git_revision_selects_only_tests_for_implementation_changes() {
    let fixture = fixture(false, false);
    run_git(fixture.path(), &["init", "-q"]);
    run_git(
        fixture.path(),
        &["config", "user.email", "walaru@example.invalid"],
    );
    run_git(fixture.path(), &["config", "user.name", "Walaru Test"]);
    run_git(
        fixture.path(),
        &[
            "add",
            "src/main/kotlin/demo/Example.kt",
            "settings.gradle.kts",
        ],
    );
    run_git(fixture.path(), &["commit", "-qm", "baseline"]);

    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));
    verifier.verify(&VerificationRequest::fast()).unwrap();
    fs::write(
        fixture.path().join("src/main/kotlin/demo/Example.kt"),
        "package demo\nfun answer() = 3\n",
    )
    .unwrap();

    verifier
        .verify(&VerificationRequest {
            since: Some("HEAD".into()),
            ..VerificationRequest::fast()
        })
        .unwrap();

    let arguments = fs::read_to_string(fixture.path().join(".gradle/last-args")).unwrap();
    assert!(
        arguments.contains("-Dwalaru.tests=demo.ExampleTest.works"),
        "--since did not select the impacted test:\n{arguments}"
    );

    fs::write(
        fixture.path().join("src/main/kotlin/demo/NewThing.kt"),
        "package demo\nfun newThing() = 1\n",
    )
    .unwrap();
    verifier
        .verify(&VerificationRequest {
            since: Some("HEAD".into()),
            ..VerificationRequest::fast()
        })
        .unwrap();
    let expanded = fs::read_to_string(fixture.path().join(".gradle/last-args")).unwrap();
    assert!(
        !expanded.contains("-Dwalaru.tests="),
        "untracked source must expand --since to all tests:\n{expanded}"
    );
}

#[test]
fn failed_test_is_structured_and_full_recording_is_replayable() {
    let fixture = fixture(false, true);
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));
    let outcome = verifier.verify(&VerificationRequest::fast()).unwrap();
    assert_eq!(outcome.status, RunStatus::Failed);
    assert_eq!(outcome.exit_code, 1);
    assert!(store.failure(&outcome.failures[0]).unwrap().is_some());

    let recording = verifier.record("demo.ExampleTest#works").unwrap();
    assert_eq!(recording.test_id, "demo.ExampleTest#works");
    assert!(
        recording
            .capabilities
            .supported
            .contains(&"singleThread".into())
    );
    assert!(recording.capabilities.supported.contains(&"pureJvm".into()));
    assert!(store.recording(&recording.id).unwrap().is_some());

    let replay_run = verifier
        .verify_replay_event(&recording, &recording.events[1].id)
        .unwrap();
    assert_ne!(replay_run, outcome.run_id);
}

#[test]
fn recorded_nondeterministic_inputs_are_reconstructed_for_fresh_replay() {
    let fixture = nondeterministic_fixture();
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));

    let recording = verifier.record("demo.InputTest#works").unwrap();
    assert_eq!(
        recording
            .events
            .iter()
            .filter(|event| event.kind == walaru_core::replay::EventKind::Input)
            .count(),
        1
    );
    assert!(
        recording
            .capabilities
            .supported
            .contains(&"deterministicInputs".into())
    );
    assert!(recording.capabilities.unavailable.is_empty());

    verifier
        .verify_replay_event(&recording, &recording.events.last().unwrap().id)
        .unwrap();
    assert_eq!(
        fs::read_to_string(fixture.path().join(".gradle/replayed-inputs")).unwrap(),
        "time.currentTimeMillis\tMTIzNDU=\n"
    );
}

#[test]
fn uniquely_named_thread_schedule_is_complete_and_passed_to_fresh_replay() {
    let fixture = scheduled_threads_fixture();
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));

    let recording = verifier.record("demo.ThreadTest#works").unwrap();
    for capability in ["threads", "threadSchedule"] {
        assert!(
            recording
                .capabilities
                .supported
                .contains(&capability.to_owned()),
            "missing {capability}: {:?}",
            recording.capabilities
        );
    }
    assert!(recording.capabilities.unavailable.is_empty());

    verifier
        .verify_replay_event(&recording, &recording.events.last().unwrap().id)
        .unwrap();
    let schedule = fs::read_to_string(fixture.path().join(".gradle/replayed-schedule")).unwrap();
    assert_eq!(schedule.lines().count(), recording.events.len());
    assert!(schedule.contains("LINE\t706c6174666f726d3a776f726b65722d61"));
    assert!(schedule.contains("LINE\t706c6174666f726d3a776f726b65722d62"));
}

#[test]
fn memory_events_preserve_watchpoint_metadata_and_capabilities() {
    let fixture = memory_events_fixture();
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));

    let recording = verifier.record("demo.MemoryTest#works").unwrap();
    for capability in [
        "fieldReads",
        "arrayWrites",
        "monitorOrder",
        "volatileAccess",
    ] {
        assert!(
            recording
                .capabilities
                .supported
                .contains(&capability.to_owned()),
            "missing {capability}: {:?}",
            recording.capabilities
        );
    }
    let array_write = recording
        .events
        .iter()
        .find(|event| {
            event.kind == walaru_core::replay::EventKind::Write
                && event.values["targetKind"] == "array"
        })
        .unwrap();
    assert_eq!(array_write.values["index"], 1);
    assert_eq!(array_write.values["value"], 7);
    assert!(recording.events.iter().any(|event| {
        event.kind == walaru_core::replay::EventKind::Monitor && event.values["action"] == "enter"
    }));
    assert_eq!(recording.checkpoints.first().unwrap().sequence, 0);
    assert_eq!(
        recording.checkpoints.last().unwrap().sequence,
        recording.events.last().unwrap().sequence
    );
    assert!(
        recording
            .capabilities
            .supported
            .contains(&"replayCheckpoints".into())
    );
}

#[test]
fn explicitly_captured_file_input_replays_without_exposing_its_bytes_in_trace() {
    let fixture = captured_file_input_fixture();
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));

    let recording = verifier
        .record_with_options(
            "demo.FileInputTest#works",
            RecordingOptions {
                capture_file_io: true,
            },
        )
        .unwrap();

    assert!(recording.capabilities.unavailable.is_empty());
    assert!(
        recording
            .capabilities
            .supported
            .contains(&"fileInputs".into())
    );
    assert_eq!(recording.inputs.len(), 1);
    assert!(recording.inputs[0].sensitive);
    let input_event = recording
        .events
        .iter()
        .find(|event| event.kind == walaru_core::replay::EventKind::Input)
        .unwrap();
    assert_eq!(
        input_event.values["value"],
        "<redacted:file-input 14 bytes>"
    );
    assert!(input_event.values.get("encoded").is_none());

    verifier
        .verify_replay_event(&recording, &recording.events.last().unwrap().id)
        .unwrap();
    assert_eq!(
        fs::read_to_string(fixture.path().join(".gradle/replayed-file-inputs")).unwrap(),
        "io.file.readString.abc\tc2VjcmV0LWNvbnRlbnQ=\n"
    );
}

#[test]
fn zero_config_maven_surefire_uses_the_same_agent_and_structured_store() {
    let fixture = maven_fixture();
    let layout = WorkspaceLayout::new(fixture.path()).unwrap();
    layout.ensure_state_dir().unwrap();
    let store = Store::open(&layout.database, RetentionPolicy::default()).unwrap();
    let verifier = Verifier::new(&layout, &store, artifacts(fixture.path()))
        .with_timeout(Duration::from_secs(10));

    let outcome = verifier.verify(&VerificationRequest::fast()).unwrap();

    assert_eq!(outcome.status, RunStatus::Passed);
    assert_eq!(outcome.tests, vec!["demo.MavenTest#works"]);
    assert_eq!(
        store
            .coverage("src/main/java/demo/Maven.java", None, 100)
            .unwrap()
            .items
            .len(),
        1
    );
    let arguments = fs::read_to_string(fixture.path().join("target/walaru-args")).unwrap();
    assert!(
        arguments.contains("-Dmaven.test.additionalClasspath="),
        "{arguments}"
    );
    assert!(arguments.contains("-javaagent:"), "{arguments}");
}

fn fixture(stale: bool, failed: bool) -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/kotlin/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/kotlin/demo/Example.kt"),
        "package demo\nfun answer() = 1\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"fake\"\n",
    )
    .unwrap();
    let finish = if failed {
        r#"{"schemaVersion":1,"sequence":2,"threadId":1,"type":"TEST_FINISH","testId":"junit-id","testName":"demo.ExampleTest#works","status":"failed","failureType":"java.lang.AssertionError","stateHash":"s2"}"#
    } else {
        r#"{"schemaVersion":1,"sequence":2,"threadId":1,"type":"TEST_FINISH","testId":"junit-id","testName":"demo.ExampleTest#works","status":"successful","stateHash":"s2"}"#
    };
    let edit = if stale {
        "printf 'package demo\\nfun answer() = 2\\n' > src/main/kotlin/demo/Example.kt"
    } else {
        ":"
    };
    let process_exit = i32::from(failed);
    let script = format!(
        r#"#!/usr/bin/env bash
set -eu
event_file=""
for argument in "$@"; do
  case "$argument" in
    -Dwalaru.eventFile=*) event_file="${{argument#*=}}" ;;
  esac
done
mkdir -p "$(dirname "$event_file")"
mkdir -p .gradle
printf x >> .gradle/fake-invocations
printf '%s\n' "$@" > .gradle/last-args
printf '%s\n' \
'{{"schemaVersion":1,"sequence":0,"threadId":1,"type":"TEST_START","testId":"junit-id","testName":"demo.ExampleTest#works","stateHash":"s0"}}' \
'{{"schemaVersion":1,"sequence":1,"threadId":1,"type":"LINE","testId":"junit-id","testName":"demo.ExampleTest#works","owner":"demo/ExampleKt","method":"answer","path":"Example.kt","line":2,"values":{{"counter":1}},"stateHash":"s1"}}' \
'{finish}' > "$event_file"
{edit}
exit {process_exit}
"#,
    );
    let wrapper = directory.path().join("gradlew");
    fs::write(&wrapper, script).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    directory
}

fn multi_module_fixture() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    for module in ["alpha", "beta"] {
        fs::create_dir_all(directory.path().join(module).join("src/main/kotlin/demo")).unwrap();
        fs::write(
            directory
                .path()
                .join(module)
                .join("src/main/kotlin/demo/Production.kt"),
            format!("package demo\nfun {module}Answer() = 1\n"),
        )
        .unwrap();
    }
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"multi\"\ninclude(\":alpha\", \":beta\")\n",
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
printf '%s\n' "$@" > .gradle/last-args
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"module":":alpha","type":"TEST_START","testId":"alpha-id","testName":":alpha::demo.SharedTest#works","stateHash":"a0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"module":":alpha","type":"LINE","testId":"alpha-id","testName":":alpha::demo.SharedTest#works","owner":"demo/ProductionKt","method":"alphaAnswer","path":"alpha/src/main/kotlin/demo/Production.kt","line":2,"stateHash":"a1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"module":":alpha","type":"TEST_FINISH","testId":"alpha-id","testName":":alpha::demo.SharedTest#works","status":"successful","stateHash":"a2"}' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"module":":beta","type":"TEST_START","testId":"beta-id","testName":":beta::demo.SharedTest#works","stateHash":"b0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"module":":beta","type":"LINE","testId":"beta-id","testName":":beta::demo.SharedTest#works","owner":"demo/ProductionKt","method":"betaAnswer","path":"beta/src/main/kotlin/demo/Production.kt","line":2,"stateHash":"b1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"module":":beta","type":"TEST_FINISH","testId":"beta-id","testName":":beta::demo.SharedTest#works","status":"successful","stateHash":"b2"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    directory
}

fn nondeterministic_fixture() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/java/demo/Input.java"),
        "package demo; final class Input { long now() { return System.currentTimeMillis(); } }\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"input\"\n",
    )
    .unwrap();
    let wrapper = directory.path().join("gradlew");
    fs::write(
        &wrapper,
        r#"#!/usr/bin/env bash
set -eu
event_file=""
replay_file=""
for argument in "$@"; do
  case "$argument" in
    -Dwalaru.eventFile=*) event_file="${argument#*=}" ;;
    -Dwalaru.replayInputFile=*) replay_file="${argument#*=}" ;;
  esac
done
mkdir -p "$(dirname "$event_file")" .gradle
if [ -n "$replay_file" ]; then cp "$replay_file" .gradle/replayed-inputs; fi
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"type":"TEST_START","testName":"demo.InputTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"type":"CALL","testName":"demo.InputTest#works","targetOwner":"java/lang/System","targetMethod":"currentTimeMillis","stateHash":"s1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"type":"INPUT","testName":"demo.InputTest#works","values":{"kind":"time.currentTimeMillis","encoded":"MTIzNDU=","value":"12345"},"stateHash":"s2"}' \
'{"schemaVersion":1,"sequence":3,"threadId":1,"type":"LINE","testName":"demo.InputTest#works","owner":"demo/Input","method":"now","path":"src/main/java/demo/Input.java","line":1,"stateHash":"s3"}' \
'{"schemaVersion":1,"sequence":4,"threadId":1,"type":"TEST_FINISH","testName":"demo.InputTest#works","status":"successful","stateHash":"s4"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    directory
}

fn scheduled_threads_fixture() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/java/demo/Threads.java"),
        "package demo; final class Threads { int value; }\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"threads\"\n",
    )
    .unwrap();
    let wrapper = directory.path().join("gradlew");
    fs::write(
        &wrapper,
        r#"#!/usr/bin/env bash
set -eu
event_file=""
schedule_file=""
for argument in "$@"; do
  case "$argument" in
    -Dwalaru.eventFile=*) event_file="${argument#*=}" ;;
    -Dwalaru.replayScheduleFile=*) schedule_file="${argument#*=}" ;;
  esac
done
mkdir -p "$(dirname "$event_file")" .gradle
if [ -n "$schedule_file" ]; then cp "$schedule_file" .gradle/replayed-schedule; fi
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"threadKey":"platform:test-worker","virtualThread":false,"type":"TEST_START","testName":"demo.ThreadTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":2,"threadKey":"platform:worker-a","virtualThread":false,"type":"LINE","testName":"demo.ThreadTest#works","owner":"demo/Threads","method":"a","path":"src/main/java/demo/Threads.java","line":1,"stateHash":"s1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":3,"threadKey":"platform:worker-b","virtualThread":false,"type":"LINE","testName":"demo.ThreadTest#works","owner":"demo/Threads","method":"b","path":"src/main/java/demo/Threads.java","line":1,"stateHash":"s2"}' \
'{"schemaVersion":1,"sequence":3,"threadId":1,"threadKey":"platform:test-worker","virtualThread":false,"type":"TEST_FINISH","testName":"demo.ThreadTest#works","status":"successful","stateHash":"s3"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    directory
}

fn maven_fixture() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/java/demo/Maven.java"),
        "package demo; public final class Maven { public int answer() { return 42; } }\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>demo</groupId><artifactId>maven</artifactId><version>1</version></project>\n",
    )
    .unwrap();
    let wrapper = directory.path().join("mvnw");
    fs::write(
        &wrapper,
        r#"#!/usr/bin/env bash
set -eu
event_file=""
for argument in "$@"; do
  case "$argument" in -Dwalaru.eventFile=*) event_file="${argument#*=}" ;; esac
done
mkdir -p "$(dirname "$event_file")" target
printf '%s\n' "$@" > target/walaru-args
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"type":"TEST_START","testName":"demo.MavenTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"type":"LINE","testName":"demo.MavenTest#works","owner":"demo/Maven","method":"answer","path":"src/main/java/demo/Maven.java","line":1,"stateHash":"s1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"type":"TEST_FINISH","testName":"demo.MavenTest#works","status":"successful","stateHash":"s2"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    directory
}

fn memory_events_fixture() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/java/demo/Memory.java"),
        "package demo; final class Memory { static volatile int value; }\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"memory\"\n",
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
'{"schemaVersion":1,"sequence":0,"threadId":1,"threadKey":"platform:main","type":"TEST_START","testName":"demo.MemoryTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"threadKey":"platform:main","type":"MONITOR","testName":"demo.MemoryTest#works","owner":"demo/Memory","method":"run","path":"src/main/java/demo/Memory.java","line":1,"values":{"action":"enter","monitorKind":"block"},"stateHash":"s1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"threadKey":"platform:main","type":"WRITE","testName":"demo.MemoryTest#works","owner":"demo/Memory","method":"run","path":"src/main/java/demo/Memory.java","line":1,"volatile":false,"values":{"targetKind":"array","targetOwner":"int[]","index":1,"value":7},"stateHash":"s2"}' \
'{"schemaVersion":1,"sequence":3,"threadId":1,"threadKey":"platform:main","type":"READ","testName":"demo.MemoryTest#works","owner":"demo/Memory","method":"run","path":"src/main/java/demo/Memory.java","line":1,"volatile":true,"values":{"targetKind":"field","field":"value","volatile":true,"value":7},"stateHash":"s3"}' \
'{"schemaVersion":1,"sequence":4,"threadId":1,"threadKey":"platform:main","type":"MONITOR","testName":"demo.MemoryTest#works","owner":"demo/Memory","method":"run","path":"src/main/java/demo/Memory.java","line":1,"values":{"action":"exit","monitorKind":"block"},"stateHash":"s4"}' \
'{"schemaVersion":1,"sequence":5,"threadId":1,"threadKey":"platform:main","type":"TEST_FINISH","testName":"demo.MemoryTest#works","status":"successful","stateHash":"s5"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    directory
}

fn captured_file_input_fixture() -> tempfile::TempDir {
    let directory = tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/main/java/demo")).unwrap();
    fs::write(
        directory.path().join("src/main/java/demo/FileInput.java"),
        "package demo; final class FileInput {}\n",
    )
    .unwrap();
    fs::write(
        directory.path().join("settings.gradle.kts"),
        "rootProject.name=\"file-input\"\n",
    )
    .unwrap();
    let wrapper = directory.path().join("gradlew");
    fs::write(
        &wrapper,
        r#"#!/usr/bin/env bash
set -eu
event_file=""
input_file=""
replay_file=""
capture="false"
for argument in "$@"; do
  case "$argument" in
    -Dwalaru.eventFile=*) event_file="${argument#*=}" ;;
    -Dwalaru.inputFile=*) input_file="${argument#*=}" ;;
    -Dwalaru.replayInputFile=*) replay_file="${argument#*=}" ;;
    -Dwalaru.captureFileIo=true) capture="true" ;;
  esac
done
test "$capture" = true
mkdir -p "$(dirname "$event_file")" .gradle
if [ -n "$input_file" ]; then printf 'io.file.readString.abc\tc2VjcmV0LWNvbnRlbnQ=\n' > "$input_file"; fi
if [ -n "$replay_file" ]; then cp "$replay_file" .gradle/replayed-file-inputs; fi
printf '%s\n' \
'{"schemaVersion":1,"sequence":0,"threadId":1,"threadKey":"platform:main","type":"TEST_START","testName":"demo.FileInputTest#works","stateHash":"s0"}' \
'{"schemaVersion":1,"sequence":1,"threadId":1,"threadKey":"platform:main","type":"CALL","testName":"demo.FileInputTest#works","targetOwner":"java/nio/file/Files","targetMethod":"readString","stateHash":"s1"}' \
'{"schemaVersion":1,"sequence":2,"threadId":1,"threadKey":"platform:main","type":"INPUT","testName":"demo.FileInputTest#works","values":{"kind":"io.file.readString.abc","sensitive":true,"value":"<redacted:file-input 14 bytes>"},"stateHash":"s2"}' \
'{"schemaVersion":1,"sequence":3,"threadId":1,"threadKey":"platform:main","type":"TEST_FINISH","testName":"demo.FileInputTest#works","status":"successful","stateHash":"s3"}' > "$event_file"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    directory
}

fn artifacts(root: &std::path::Path) -> RuntimeArtifacts {
    for file in ["adapter.jar", "agent.jar", "init.gradle.kts"] {
        fs::write(root.join(file), "fixture").unwrap();
    }
    RuntimeArtifacts {
        adapter_jar: root.join("adapter.jar"),
        agent_jar: root.join("agent.jar"),
        init_script: root.join("init.gradle.kts"),
    }
}

fn assert_only_run_is_error(layout: &WorkspaceLayout, store: &Store) {
    let run_directory = layout.state_dir.join("runs");
    let run_id = fs::read_dir(run_directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        store.run(&run_id).unwrap().unwrap().status,
        RunStatus::Error
    );
}

fn run_git(root: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
