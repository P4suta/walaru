//! Frozen JSON, protobuf, identity, and replay contracts.

use std::collections::BTreeMap;

use prost::Message;
use serde_json::{Value, json};
use walaru_core::event::{EventId, EventIdentity, RevisionId, WorkspaceId};
use walaru_core::protocol::{
    CapabilityManifest, Completeness, Diagnostic, Envelope, NextAction, Page, RpcRequest,
    RpcResponse, Status,
};
use walaru_core::replay::{
    Event, EventKind, JvmReplayBackend, LinuxProcessReplayArtifact, LinuxProcessReplayBackend,
    Recording, ReplayBackend, ReplayError, ReverseRequest, StepKind, verify_replayed_prefix,
};

#[test]
fn stable_ids_are_canonical_and_revision_bound() {
    let unix = WorkspaceId::from_path("/work/project");
    let windows_separators = WorkspaceId::from_path("\\work\\project\\");
    assert_eq!(unix, windows_separators);
    assert!(unix.as_str().starts_with("ws-"));

    let revision = RevisionId::from_digest([0x2a; 32]);
    let same = EventId::new(&EventIdentity {
        revision: revision.clone(),
        run_id: "run-1".into(),
        test_id: "demo.ExampleTest#works".into(),
        sequence: 7,
        thread_id: 1,
    });
    let repeated = EventId::new(&EventIdentity {
        revision: revision.clone(),
        run_id: "run-1".into(),
        test_id: "demo.ExampleTest#works".into(),
        sequence: 7,
        thread_id: 1,
    });
    let next_revision = EventId::new(&EventIdentity {
        revision: RevisionId::from_digest([0x2b; 32]),
        run_id: "run-1".into(),
        test_id: "demo.ExampleTest#works".into(),
        sequence: 7,
        thread_id: 1,
    });

    assert_eq!(same, repeated);
    assert_ne!(same, next_revision);
    assert!(same.as_str().starts_with("evt-"));
}

#[test]
fn json_envelope_matches_the_frozen_golden_contract() {
    let envelope = Envelope {
        schema_version: "1".into(),
        workspace_id: "ws-0123456789abcdef".into(),
        revision: "rev-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        session_id: "session-1".into(),
        run_id: Some("run-1".into()),
        status: Status::Ok,
        data: json!({"tests": [{"id": "demo.ExampleTest#works", "status": "passed"}]}),
        diagnostics: vec![Diagnostic {
            code: "WALARU_INFO".into(),
            severity: "info".into(),
            message: "fixture".into(),
            details: BTreeMap::new(),
        }],
        capabilities: CapabilityManifest {
            backend: "jvm".into(),
            completeness: Completeness::Complete,
            supported: vec!["singleThread".into(), "line".into()],
            unavailable: BTreeMap::new(),
        },
        next_actions: vec![NextAction {
            title: "Inspect trace".into(),
            argv: vec!["walaru".into(), "trace".into(), "run-1".into()],
        }],
        page: Some(Page {
            cursor: None,
            next_cursor: None,
            limit: 100,
            returned: 1,
        }),
    };

    let actual = serde_json::to_value(envelope).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("../../../fixtures/golden/envelope-v1.json")).unwrap();
    assert_eq!(actual, expected);
    assert!(actual["nextActions"][0]["argv"].is_array());
}

#[test]
fn protobuf_rpc_is_versioned_and_round_trips_unknown_json_payloads() {
    let request = RpcRequest {
        schema_version: 1,
        request_id: "request-7".into(),
        workspace_root: "/work/project".into(),
        command: "coverage".into(),
        payload_json: br#"{"subject":"src/main/kotlin/demo/Example.kt","futureField":true}"#
            .to_vec(),
    };
    let bytes = request.encode_to_vec();
    let decoded = RpcRequest::decode(bytes.as_slice()).unwrap();
    assert_eq!(decoded, request);

    let response = RpcResponse {
        schema_version: 1,
        request_id: request.request_id,
        exit_code: 0,
        envelope_json: br#"{"schemaVersion":"1"}"#.to_vec(),
    };
    assert_eq!(
        RpcResponse::decode(response.encode_to_vec().as_slice()).unwrap(),
        response
    );
}

#[test]
fn jvm_reverse_step_returns_the_previous_matching_event() {
    let recording = single_thread_recording();
    let backend = JvmReplayBackend;
    let outcome = backend
        .reverse(
            &recording,
            &ReverseRequest {
                from_event_id: recording.events[3].id.clone(),
                step: Some(StepKind::Line),
                until: None,
                watch: None,
            },
        )
        .unwrap();

    assert_eq!(outcome.event.id, recording.events[1].id);
    assert_eq!(outcome.event.location.as_ref().unwrap().line, 11);
    assert_eq!(outcome.event.values["counter"], json!(1));
    assert!(
        !outcome.verified,
        "index navigation alone is not replay verification"
    );
}

#[test]
fn fresh_reexecution_must_match_every_observable_event_through_the_target() {
    let recording = single_thread_recording();
    let target = &recording.events[2];
    let replayed = recording.events.clone();

    verify_replayed_prefix(&recording, &replayed, &target.id).unwrap();

    let mut differently_timed = replayed.clone();
    differently_timed[1].observations = json!({"durationNanos": 999});
    verify_replayed_prefix(&recording, &differently_timed, &target.id).unwrap();

    let mut diverged = replayed;
    diverged[1].values = json!({"counter": 99});
    let error = verify_replayed_prefix(&recording, &diverged, &target.id).unwrap_err();
    assert!(matches!(error, ReplayError::Diverged { sequence: 1, .. }));
}

#[test]
fn malformed_recording_checkpoint_cannot_be_used_as_a_verified_anchor() {
    let mut recording = single_thread_recording();
    recording
        .checkpoints
        .push(walaru_core::replay::ReplayCheckpoint {
            sequence: recording.events[1].sequence,
            event_id: recording.events[1].id.clone(),
            state_hash: "tampered".into(),
            output_index: recording.events[1].output_index,
        });

    let error =
        verify_replayed_prefix(&recording, &recording.events, &recording.events[2].id).unwrap_err();

    assert!(matches!(error, ReplayError::Diverged { sequence: 1, .. }));
}

#[test]
fn jvm_backend_refuses_to_claim_completeness_for_native_io() {
    let mut recording = single_thread_recording();
    recording.capabilities.completeness = Completeness::Partial;
    recording
        .capabilities
        .unavailable
        .insert("native".into(), "JNI boundary observed".into());

    let error = JvmReplayBackend
        .reverse(
            &recording,
            &ReverseRequest {
                from_event_id: recording.events[3].id.clone(),
                step: Some(StepKind::Line),
                until: None,
                watch: None,
            },
        )
        .unwrap_err();
    assert!(matches!(error, ReplayError::UnsupportedCapability { .. }));
}

#[test]
fn jvm_backend_accepts_a_complete_recorded_thread_schedule() {
    let mut recording = single_thread_recording();
    recording
        .capabilities
        .supported
        .retain(|value| value != "singleThread");
    recording
        .capabilities
        .supported
        .extend(["threads".into(), "threadSchedule".into()]);
    recording.events[1].thread_id = 2;
    recording.events[1].thread_key = "platform:worker-b".into();
    recording.events[3].thread_id = 2;
    recording.events[3].thread_key = "platform:worker-b".into();

    let outcome = JvmReplayBackend
        .reverse(
            &recording,
            &ReverseRequest {
                from_event_id: recording.events[3].id.clone(),
                step: Some(StepKind::Line),
                until: None,
                watch: None,
            },
        )
        .unwrap();

    assert_eq!(outcome.event.id, recording.events[1].id);
}

#[test]
fn reverse_write_can_stop_at_an_exact_field_watchpoint() {
    let mut recording = single_thread_recording();
    recording.events[1].kind = EventKind::Write;
    recording.events[1].values = json!({
        "targetKind": "field",
        "targetOwner": "demo/Counter",
        "field": "first",
        "value": 1
    });
    recording.events[2].kind = EventKind::Write;
    recording.events[2].values = json!({
        "targetKind": "field",
        "targetOwner": "demo/Counter",
        "field": "second",
        "value": 2
    });

    let outcome = JvmReplayBackend
        .reverse(
            &recording,
            &ReverseRequest {
                from_event_id: recording.events[3].id.clone(),
                step: Some(StepKind::Write),
                until: None,
                watch: Some("demo.Counter#first".into()),
            },
        )
        .unwrap();

    assert_eq!(outcome.event.id, recording.events[1].id);
}

#[test]
fn linux_rr_adapter_builds_a_shell_free_cross_indexed_navigation_plan() {
    let mut recording = single_thread_recording();
    recording.backend = "linux-process".into();
    recording.capabilities = CapabilityManifest {
        backend: "linux-process".into(),
        completeness: Completeness::Partial,
        supported: vec!["rrAdapter".into(), "crossIndex".into()],
        unavailable: [(
            "exactStateProof".into(),
            "native state was not captured".into(),
        )]
        .into_iter()
        .collect(),
    };
    recording.linux_process = Some(LinuxProcessReplayArtifact {
        trace_directory: "/var/lib/walaru/traces/trace with spaces".into(),
        event_index: [(recording.events[1].id.clone(), 17_229)]
            .into_iter()
            .collect(),
    });

    let request = ReverseRequest {
        from_event_id: recording.events[3].id.clone(),
        step: Some(StepKind::Line),
        until: None,
        watch: None,
    };
    let backend = LinuxProcessReplayBackend::with_rr_executable("/opt/rr/bin/rr");
    let plan = backend.plan_reverse(&recording, &request).unwrap();

    assert_eq!(plan.event.id, recording.events[1].id);
    assert_eq!(plan.executable, "/opt/rr/bin/rr");
    assert_eq!(
        plan.argv,
        [
            "replay",
            "-g",
            "17229",
            "/var/lib/walaru/traces/trace with spaces"
        ]
    );
    assert!(!plan.verified, "an rr jump is not a JVM/native state proof");

    let error = backend.reverse(&recording, &request).unwrap_err();
    assert!(matches!(
        error,
        ReplayError::UnsupportedCapability { capability, .. }
            if capability == "exactStateProof"
    ));
}

#[test]
fn linux_rr_adapter_rejects_a_recording_without_an_event_cross_index() {
    let mut recording = single_thread_recording();
    recording.backend = "linux-process".into();
    recording.linux_process = Some(LinuxProcessReplayArtifact {
        trace_directory: "/trace".into(),
        event_index: BTreeMap::new(),
    });
    let error = LinuxProcessReplayBackend::with_rr_executable("rr")
        .plan_reverse(
            &recording,
            &ReverseRequest {
                from_event_id: recording.events[3].id.clone(),
                step: Some(StepKind::Line),
                until: None,
                watch: None,
            },
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ReplayError::UnsupportedCapability { capability, .. }
            if capability == "crossIndex"
    ));
}

fn single_thread_recording() -> Recording {
    let revision = RevisionId::from_digest([7; 32]);
    let identity = |sequence| EventIdentity {
        revision: revision.clone(),
        run_id: "run-1".into(),
        test_id: "demo.ExampleTest#works".into(),
        sequence,
        thread_id: 1,
    };
    Recording {
        id: "rec-1".into(),
        revision: revision.to_string(),
        test_id: "demo.ExampleTest#works".into(),
        backend: "jvm".into(),
        capabilities: CapabilityManifest {
            backend: "jvm".into(),
            completeness: Completeness::Complete,
            supported: vec!["singleThread".into(), "pureJvm".into(), "line".into()],
            unavailable: BTreeMap::new(),
        },
        inputs: Vec::new(),
        linux_process: None,
        events: vec![
            event(&identity(0), EventKind::TestStart, 10, json!({})),
            event(&identity(1), EventKind::Line, 11, json!({"counter": 1})),
            event(&identity(2), EventKind::Call, 12, json!({"counter": 2})),
            event(&identity(3), EventKind::Line, 13, json!({"counter": 3})),
        ],
        checkpoints: Vec::new(),
    }
}

fn event(identity: &EventIdentity, kind: EventKind, line: u32, values: Value) -> Event {
    Event {
        id: EventId::new(identity).to_string(),
        sequence: identity.sequence,
        thread_id: identity.thread_id,
        thread_key: "platform:test-worker".into(),
        virtual_thread: false,
        coroutine: false,
        logical_stack: Vec::new(),
        kind,
        location: Some(walaru_core::replay::SourceLocation {
            path: "src/main/kotlin/demo/Example.kt".into(),
            line,
            column: 1,
            symbol: "demo.example".into(),
        }),
        values,
        observations: json!({}),
        state_hash: format!("state-{line}"),
        output_index: 0,
    }
}
