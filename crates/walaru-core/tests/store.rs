//! Executable storage and conservative-impact contract.

use std::collections::BTreeMap;

use serde_json::json;
use tempfile::tempdir;
use time::SignedDuration as Duration;
use walaru_core::protocol::{CapabilityManifest, Completeness};
use walaru_core::replay::{Event, EventKind, Recording, SourceLocation};
use walaru_core::store::RunTimestamp;
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
            started_at: "2026-08-29T01:00:00Z".parse::<RunTimestamp>().unwrap(),
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
            started_at: RunTimestamp::now(),
        })
        .unwrap();

    let status = store
        .finish_run(
            "run-stale",
            RunStatus::Passed,
            "rev-after",
            RunTimestamp::now(),
        )
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
fn stored_utc_timestamps_round_trip_without_changing_sqlite_text() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("runs.sqlite");
    let store = Store::open(&path, RetentionPolicy::default()).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    for (input, expected) in [
        ("1970-01-01T01:00:00+01:00", "1970-01-01T00:00:00+00:00"),
        ("1969-12-31T23:59:59.1Z", "1969-12-31T23:59:59.100+00:00"),
        (
            "2026-01-01T00:00:00.1234Z",
            "2026-01-01T00:00:00.123400+00:00",
        ),
        (
            "2026-01-01T00:00:00.000000001Z",
            "2026-01-01T00:00:00.000000001+00:00",
        ),
        ("2016-12-31T23:59:60.125Z", "2016-12-31T23:59:60.125+00:00"),
    ] {
        let stamp: RunTimestamp = input.parse().unwrap();
        store
            .begin_run(&NewRun {
                id: input.into(),
                revision: "rev".into(),
                source_digest: "digest".into(),
                started_at: stamp,
            })
            .unwrap();
        let text: String = db
            .query_row(
                "SELECT started_at FROM runs WHERE id = ?1",
                [input],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(text, expected);
        db.execute("UPDATE runs SET started_at = ?1 WHERE id = ?1", [input])
            .unwrap();
        assert_eq!(store.run(input).unwrap().unwrap().started_at, stamp);
    }
}

#[test]
fn retention_arithmetic_preserves_recorded_leap_seconds() {
    let stamp = "2016-12-31T23:59:60.125Z".parse::<RunTimestamp>().unwrap();
    for (age, expected) in [
        (Duration::milliseconds(125), "2016-12-31T23:59:60+00:00"),
        (Duration::milliseconds(126), "2016-12-31T23:59:59.999+00:00"),
        (Duration::seconds(1), "2016-12-31T23:59:59.125+00:00"),
        (Duration::milliseconds(-875), "2017-01-01T00:00:00+00:00"),
    ] {
        assert_eq!((stamp - age).to_string(), expected);
    }
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
    let now = "2026-08-29T01:00:00Z".parse::<RunTimestamp>().unwrap();
    for (id, started_at) in [
        ("old", now - Duration::days(8)),
        ("boundary", now - Duration::days(7)),
        (
            "immediately-before",
            now - Duration::days(7) - Duration::nanoseconds(1),
        ),
        (
            "immediately-after",
            now - Duration::days(7) - Duration::nanoseconds(-1),
        ),
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
    assert_eq!(result.runs_removed, 2);
    assert!(store.run("old").unwrap().is_none());
    assert!(store.run("immediately-before").unwrap().is_none());
    assert!(store.run("boundary").unwrap().is_some());
    assert!(store.run("immediately-after").unwrap().is_some());
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

    let result = store.prune(RunTimestamp::now()).unwrap();

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
                started_at: RunTimestamp::now(),
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
