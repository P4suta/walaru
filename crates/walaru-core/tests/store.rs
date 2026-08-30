//! Executable storage and conservative-impact contract.

use std::collections::BTreeMap;

use chrono::{Duration, TimeZone, Utc};
use serde_json::json;
use tempfile::tempdir;
use walaru_core::protocol::{CapabilityManifest, Completeness};
use walaru_core::replay::{Event, EventKind, Recording, SourceLocation};
use walaru_core::store::{
    Dependency, ImpactSelection, NewRun, RetentionPolicy, RunStatus, Store, TestRecord,
};

#[test]
fn sqlite_store_uses_wal_and_round_trips_zstd_event_values() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("store.sqlite3");
    let store = Store::open(&path, RetentionPolicy::default()).unwrap();
    assert_eq!(store.journal_mode().unwrap().to_ascii_lowercase(), "wal");

    store
        .begin_run(&NewRun {
            id: "run-1".into(),
            revision: "rev-a".into(),
            source_digest: "digest-a".into(),
            started_at: Utc.with_ymd_and_hms(2026, 8, 29, 1, 0, 0).unwrap(),
        })
        .unwrap();
    let repeated = "safe-value-".repeat(2_000);
    store
        .append_event(
            "run-1",
            "demo.ExampleTest#works",
            &Event {
                id: "evt-1".into(),
                sequence: 1,
                thread_id: 7,
                thread_key: "platform:test-worker".into(),
                virtual_thread: false,
                coroutine: false,
                logical_stack: Vec::new(),
                kind: EventKind::Line,
                location: Some(SourceLocation {
                    path: "src/main/kotlin/demo/Example.kt".into(),
                    line: 12,
                    column: 1,
                    symbol: "demo.example".into(),
                }),
                values: json!({"message": repeated}),
                observations: json!({}),
                state_hash: "state-a".into(),
                output_index: 0,
            },
        )
        .unwrap();

    let page = store.events("run-1", None, 100, 64 * 1024).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].values["message"],
        json!("safe-value-".repeat(2_000))
    );
    assert!(page.next_cursor.is_none());
    assert!(store.compressed_value_bytes("evt-1").unwrap() < 1_000);
}

#[test]
fn changed_revision_can_never_finish_as_success() {
    let directory = tempdir().unwrap();
    let store = Store::open(
        directory.path().join("store.sqlite3"),
        RetentionPolicy::default(),
    )
    .unwrap();
    store
        .begin_run(&NewRun {
            id: "run-stale".into(),
            revision: "rev-before".into(),
            source_digest: "digest-before".into(),
            started_at: Utc::now(),
        })
        .unwrap();

    let status = store
        .finish_run("run-stale", RunStatus::Passed, "rev-after", Utc::now())
        .unwrap();
    assert_eq!(status, RunStatus::Stale);
    assert_eq!(
        store.run("run-stale").unwrap().unwrap().status,
        RunStatus::Stale
    );
}

#[test]
fn impact_is_exact_only_when_known_and_otherwise_expands_to_all_tests() {
    let directory = tempdir().unwrap();
    let store = Store::open(
        directory.path().join("store.sqlite3"),
        RetentionPolicy::default(),
    )
    .unwrap();
    for id in ["demo.FastTest#one", "demo.SlowTest#two"] {
        store
            .upsert_test(&TestRecord {
                id: id.into(),
                display_name: id.into(),
                module: ":app".into(),
                last_status: None,
                last_failure_id: None,
            })
            .unwrap();
    }
    store
        .replace_dependencies(
            "demo.FastTest#one",
            &[Dependency {
                subject: "src/main/kotlin/demo/Fast.kt".into(),
                kind: "method".into(),
            }],
        )
        .unwrap();

    assert_eq!(
        store
            .select_impact("src/main/kotlin/demo/Fast.kt", ":app")
            .unwrap(),
        ImpactSelection::Exact(vec!["demo.FastTest#one".into()])
    );
    assert_eq!(
        store.select_impact("build.gradle.kts", ":app").unwrap(),
        ImpactSelection::ModuleAll {
            tests: vec!["demo.FastTest#one".into(), "demo.SlowTest#two".into()],
            reason: "unknown dependency or build/resource/global change".into(),
        }
    );
}

#[test]
fn recordings_preserve_capability_manifest_and_event_order() {
    let directory = tempdir().unwrap();
    let store = Store::open(
        directory.path().join("store.sqlite3"),
        RetentionPolicy::default(),
    )
    .unwrap();
    let recording = Recording {
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
        events: vec![simple_event("evt-1", 1), simple_event("evt-2", 2)],
        checkpoints: Vec::new(),
    };
    store.save_recording(&recording).unwrap();
    assert_eq!(store.recording("rec-1").unwrap().unwrap(), recording);
}

#[test]
fn retention_removes_expired_runs_without_touching_recent_runs() {
    let directory = tempdir().unwrap();
    let store = Store::open(
        directory.path().join("store.sqlite3"),
        RetentionPolicy {
            max_age: Duration::days(7),
            max_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 29, 1, 0, 0).unwrap();
    for (id, started_at) in [
        ("old", now - Duration::days(8)),
        ("recent", now - Duration::days(1)),
    ] {
        store
            .begin_run(&NewRun {
                id: id.into(),
                revision: "rev-a".into(),
                source_digest: "digest-a".into(),
                started_at,
            })
            .unwrap();
    }

    let result = store.prune(now).unwrap();
    assert_eq!(result.runs_removed, 1);
    assert!(store.run("old").unwrap().is_none());
    assert!(store.run("recent").unwrap().is_some());
}

#[test]
fn retention_size_limit_removes_recordings_even_when_no_runs_remain() {
    let directory = tempdir().unwrap();
    let store = Store::open(
        directory.path().join("store.sqlite3"),
        RetentionPolicy {
            max_age: Duration::days(7),
            max_bytes: 1,
        },
    )
    .unwrap();
    let recording = Recording {
        id: "rec-too-large".into(),
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
        events: vec![simple_event("evt-large", 1)],
        checkpoints: Vec::new(),
    };
    store.save_recording(&recording).unwrap();

    let result = store.prune(Utc::now()).unwrap();

    assert_eq!(result.recordings_removed, 1);
    assert!(store.recording("rec-too-large").unwrap().is_none());
}

#[test]
fn reopening_after_a_crash_marks_orphaned_running_runs_as_error() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("store.sqlite3");
    {
        let store = Store::open(&database, RetentionPolicy::default()).unwrap();
        store
            .begin_run(&NewRun {
                id: "orphaned".into(),
                revision: "rev-a".into(),
                source_digest: "digest-a".into(),
                started_at: Utc::now(),
            })
            .unwrap();
        assert_eq!(
            store.run("orphaned").unwrap().unwrap().status,
            RunStatus::Running
        );
    }

    let recovered = Store::open(&database, RetentionPolicy::default()).unwrap();
    let run = recovered.run("orphaned").unwrap().unwrap();

    assert_eq!(run.status, RunStatus::Error);
    assert!(run.finished_at.is_some());
}

fn simple_event(id: &str, sequence: u64) -> Event {
    Event {
        id: id.into(),
        sequence,
        thread_id: 1,
        thread_key: "platform:test-worker".into(),
        virtual_thread: false,
        coroutine: false,
        logical_stack: Vec::new(),
        kind: EventKind::Line,
        location: None,
        values: json!({"sequence": sequence}),
        observations: json!({}),
        state_hash: format!("state-{sequence}"),
        output_index: 0,
    }
}
