//! Daemon query envelopes and reverse capability exit codes.

use std::collections::BTreeMap;

use chrono::Utc;
use serde_json::{Value, json};
use tempfile::tempdir;
use walaru_core::protocol::{CapabilityManifest, Completeness, Envelope, RpcRequest};
use walaru_core::replay::{Event, EventKind, Recording};
use walaru_core::store::{FailureRecord, NewRun, TestRecord};
use walaru_daemon::Daemon;

#[test]
fn tests_values_and_reverse_are_returned_through_the_fixed_envelope() {
    let directory = tempdir().unwrap();
    let daemon = Daemon::open(directory.path()).unwrap();
    daemon
        .store()
        .begin_run(&NewRun {
            id: "run-1".into(),
            revision: "rev-a".into(),
            source_digest: "digest-a".into(),
            started_at: Utc::now(),
        })
        .unwrap();
    daemon
        .store()
        .upsert_test(&TestRecord {
            id: "demo.ExampleTest#works".into(),
            display_name: "works".into(),
            module: ":".into(),
            last_status: Some("passed".into()),
            last_failure_id: None,
        })
        .unwrap();
    let events = vec![
        event("evt-1", 1, EventKind::Line),
        event("evt-2", 2, EventKind::Line),
    ];
    for event in &events {
        daemon
            .store()
            .append_event("run-1", "demo.ExampleTest#works", event)
            .unwrap();
    }
    daemon
        .store()
        .save_recording(&Recording {
            id: "rec-1".into(),
            revision: "rev-a".into(),
            test_id: "demo.ExampleTest#works".into(),
            backend: "jvm".into(),
            capabilities: CapabilityManifest {
                backend: "jvm".into(),
                completeness: Completeness::Complete,
                supported: vec!["singleThread".into(), "pureJvm".into()],
                unavailable: BTreeMap::new(),
            },
            inputs: Vec::new(),
            linux_process: None,
            events,
            checkpoints: Vec::new(),
        })
        .unwrap();

    let tests = envelope(daemon.handle(request(directory.path(), "tests", json!({}), 1)));
    assert_eq!(tests.data["tests"][0]["id"], "demo.ExampleTest#works");
    assert_eq!(tests.page.unwrap().returned, 1);

    let filtered = envelope(daemon.handle(request_with_fields(
        directory.path(),
        "tests",
        json!({}),
        1,
        &["tests.id"],
    )));
    assert_eq!(filtered.data["tests"][0]["id"], "demo.ExampleTest#works");
    assert!(filtered.data["tests"][0].get("displayName").is_none());
    assert!(filtered.data["tests"][0].get("lastStatus").is_none());

    let mut historical = request(directory.path(), "tests", json!({}), 1);
    let mut historical_payload: Value = serde_json::from_slice(&historical.payload_json).unwrap();
    historical_payload["query"]["at"] = json!(format!("rev-{}", "f".repeat(64)));
    historical.payload_json = serde_json::to_vec(&historical_payload).unwrap();
    let historical = daemon.handle(historical);
    assert_eq!(historical.exit_code, 4);
    assert_eq!(
        envelope(historical).diagnostics[0].code,
        "WALARU_QUERY_REVISION"
    );

    let values = envelope(daemon.handle(request(
        directory.path(),
        "values",
        json!({"event": "evt-2"}),
        100,
    )));
    assert_eq!(values.data["values"]["counter"], 2);

    daemon
        .store()
        .save_failure(&FailureRecord {
            id: "failure-1".into(),
            run_id: "run-1".into(),
            test_id: "demo.ExampleTest#works".into(),
            exception_type: "java.lang.AssertionError".into(),
            message: "expected 1".into(),
            event_id: Some("evt-2".into()),
            frames: Vec::new(),
        })
        .unwrap();
    let tests = envelope(daemon.handle(request(directory.path(), "tests", json!({}), 100)));
    assert_eq!(tests.data["tests"][0]["lastFailureId"], "failure-1");
    let failure = envelope(daemon.handle(request(
        directory.path(),
        "failure",
        json!({"id": "failure-1"}),
        100,
    )));
    assert_eq!(
        failure.next_actions[0].argv,
        [
            "walaru",
            "trace",
            "demo.ExampleTest#works",
            "--format",
            "json"
        ]
    );
    assert_eq!(
        failure.next_actions[1].argv,
        ["walaru", "values", "evt-2", "--format", "json"]
    );

    let reverse = daemon.handle(request(
        directory.path(),
        "reverse",
        json!({"recordingId": "rec-1", "from": "evt-2", "step": "line", "until": null}),
        100,
    ));
    assert_eq!(reverse.exit_code, 4);
    let reverse = envelope(reverse);
    assert_eq!(reverse.data["verified"], false);
    assert_eq!(reverse.diagnostics[0].code, "WALARU_REPLAY_UNVERIFIED");
}

#[test]
fn incomplete_recording_returns_capability_exit_four() {
    let directory = tempdir().unwrap();
    let daemon = Daemon::open(directory.path()).unwrap();
    daemon
        .store()
        .save_recording(&Recording {
            id: "rec-partial".into(),
            revision: "rev-a".into(),
            test_id: "demo.ExampleTest#works".into(),
            backend: "jvm".into(),
            capabilities: CapabilityManifest {
                backend: "jvm".into(),
                completeness: Completeness::Partial,
                supported: vec!["line".into()],
                unavailable: [("native".into(), "JNI boundary observed".into())]
                    .into_iter()
                    .collect(),
            },
            inputs: Vec::new(),
            linux_process: None,
            events: vec![
                event("evt-1", 1, EventKind::Line),
                event("evt-2", 2, EventKind::Line),
            ],
            checkpoints: Vec::new(),
        })
        .unwrap();
    let response = daemon.handle(request(
        directory.path(),
        "reverse",
        json!({"recordingId": "rec-partial", "from": "evt-2", "step": "line", "until": null}),
        100,
    ));
    assert_eq!(response.exit_code, 4);
    let envelope = envelope(response);
    assert_eq!(envelope.status, walaru_core::protocol::Status::Unsupported);
    assert_eq!(envelope.diagnostics[0].code, "WALARU_REPLAY_CAPABILITY");
}

#[test]
fn oversized_single_event_never_breaks_the_default_json_response_bound() {
    let directory = tempdir().unwrap();
    let daemon = Daemon::open(directory.path()).unwrap();
    daemon
        .store()
        .begin_run(&NewRun {
            id: "run-large".into(),
            revision: "rev-a".into(),
            source_digest: "digest-a".into(),
            started_at: Utc::now(),
        })
        .unwrap();
    let mut oversized = event("evt-large", 1, EventKind::Line);
    oversized.values = json!({"payload": "x".repeat(128 * 1024)});
    daemon
        .store()
        .append_event("run-large", "demo.LargeTest#works", &oversized)
        .unwrap();

    let response = daemon.handle(request(
        directory.path(),
        "trace",
        json!({"subject": "run-large"}),
        100,
    ));

    assert!(response.envelope_json.len() <= 65_536);
    assert_eq!(response.exit_code, 4);
    let envelope = envelope(response);
    assert_eq!(envelope.status, walaru_core::protocol::Status::Partial);
    assert_eq!(envelope.data["truncated"], true);
    assert_eq!(envelope.diagnostics[0].code, "WALARU_RESPONSE_LIMIT");
}

#[test]
fn trace_field_projection_is_paged_by_the_projected_size() {
    let directory = tempdir().unwrap();
    let daemon = Daemon::open(directory.path()).unwrap();
    daemon
        .store()
        .begin_run(&NewRun {
            id: "run-projected".into(),
            revision: "rev-a".into(),
            source_digest: "digest-a".into(),
            started_at: Utc::now(),
        })
        .unwrap();
    for sequence in 1..=3 {
        let mut oversized = event(
            &format!("evt-projected-{sequence}"),
            sequence,
            EventKind::Line,
        );
        oversized.values = json!({"payload": "x".repeat(48 * 1024)});
        daemon
            .store()
            .append_event("run-projected", "demo.LargeTest#works", &oversized)
            .unwrap();
    }

    let response = daemon.handle(request_with_fields(
        directory.path(),
        "trace",
        json!({"subject": "run-projected"}),
        3,
        &["events.id"],
    ));

    assert_eq!(response.exit_code, 0);
    assert!(response.envelope_json.len() <= 65_536);
    let envelope = envelope(response);
    assert_eq!(envelope.page.as_ref().unwrap().returned, 3);
    assert!(envelope.page.as_ref().unwrap().next_cursor.is_none());
    assert_eq!(envelope.data["events"].as_array().unwrap().len(), 3);
    assert!(envelope.data["events"][0].get("values").is_none());
}

#[allow(clippy::needless_pass_by_value)]
fn request(
    workspace: &std::path::Path,
    command: &str,
    command_data: Value,
    limit: usize,
) -> RpcRequest {
    RpcRequest {
        schema_version: 1,
        request_id: format!("request-{command}"),
        workspace_root: workspace.to_string_lossy().into_owned(),
        command: command.into(),
        payload_json: serde_json::to_vec(&json!({
            "query": {"fields": [], "limit": limit, "cursor": null, "maxBytes": 65536, "at": null},
            "command": command_data,
        }))
        .unwrap(),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn request_with_fields(
    workspace: &std::path::Path,
    command: &str,
    command_data: Value,
    limit: usize,
    fields: &[&str],
) -> RpcRequest {
    RpcRequest {
        schema_version: 1,
        request_id: format!("request-{command}-fields"),
        workspace_root: workspace.to_string_lossy().into_owned(),
        command: command.into(),
        payload_json: serde_json::to_vec(&json!({
            "query": {
                "fields": fields,
                "limit": limit,
                "cursor": null,
                "maxBytes": 65536,
                "at": null,
            },
            "command": command_data,
        }))
        .unwrap(),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn envelope(response: walaru_core::protocol::RpcResponse) -> Envelope {
    serde_json::from_slice(&response.envelope_json).unwrap()
}

fn event(id: &str, sequence: u64, kind: EventKind) -> Event {
    Event {
        id: id.into(),
        sequence,
        thread_id: 1,
        thread_key: "platform:test-worker".into(),
        virtual_thread: false,
        coroutine: false,
        logical_stack: Vec::new(),
        kind,
        location: None,
        values: json!({"counter": sequence}),
        state_hash: format!("state-{sequence}"),
        output_index: 0,
    }
}
