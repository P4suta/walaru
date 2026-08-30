//! Query model and pagination contract for AI clients.

use chrono::Utc;
use serde_json::json;
use tempfile::tempdir;
use walaru_core::replay::{Event, EventKind, SourceLocation};
use walaru_core::store::{
    CoverageRecord, FailureRecord, NewRun, RetentionPolicy, RunStatus, Store, TestRecord,
};

#[test]
fn tests_failures_coverage_trace_and_values_are_structured_and_bounded() {
    let directory = tempdir().unwrap();
    let store = Store::open(
        directory.path().join("store.sqlite3"),
        RetentionPolicy::default(),
    )
    .unwrap();
    store
        .begin_run(&NewRun {
            id: "run-queries".into(),
            revision: "rev-query".into(),
            source_digest: "digest-query".into(),
            started_at: Utc::now(),
        })
        .unwrap();
    for (id, status) in [
        ("demo.AlphaTest#works", "passed"),
        ("demo.BetaTest#fails", "failed"),
    ] {
        store
            .upsert_test(&TestRecord {
                id: id.into(),
                display_name: id.into(),
                module: ":app".into(),
                last_status: Some(status.into()),
                last_failure_id: None,
            })
            .unwrap();
    }
    let unrelated_event = Event {
        id: "evt-alpha".into(),
        sequence: 1,
        thread_id: 1,
        thread_key: "platform:test-worker".into(),
        virtual_thread: false,
        coroutine: false,
        logical_stack: Vec::new(),
        kind: EventKind::Line,
        location: Some(SourceLocation {
            path: "src/main/kotlin/demo/Alpha.kt".into(),
            line: 8,
            column: 1,
            symbol: "demo.alpha".into(),
        }),
        values: json!({"unrelated": true}),
        state_hash: "state-alpha".into(),
        output_index: 0,
    };
    store
        .append_event("run-queries", "demo.AlphaTest#works", &unrelated_event)
        .unwrap();

    let event = Event {
        id: "evt-failure".into(),
        sequence: 9,
        thread_id: 1,
        thread_key: "platform:test-worker".into(),
        virtual_thread: false,
        coroutine: false,
        logical_stack: Vec::new(),
        kind: EventKind::Line,
        location: Some(SourceLocation {
            path: "src/main/kotlin/demo/Beta.kt".into(),
            line: 21,
            column: 1,
            symbol: "demo.beta".into(),
        }),
        values: json!({"actual": 2, "expected": 3}),
        state_hash: "state-failure".into(),
        output_index: 0,
    };
    store
        .append_event("run-queries", "demo.BetaTest#fails", &event)
        .unwrap();
    store
        .save_failure(&FailureRecord {
            id: "failure-1".into(),
            run_id: "run-queries".into(),
            test_id: "demo.BetaTest#fails".into(),
            exception_type: "java.lang.AssertionError".into(),
            message: "expected 3 but was 2".into(),
            event_id: Some("evt-failure".into()),
            frames: vec!["demo.BetaTest.fails(BetaTest.kt:21)".into()],
        })
        .unwrap();
    store
        .replace_coverage(
            "demo.BetaTest#fails",
            &[CoverageRecord {
                test_id: "demo.BetaTest#fails".into(),
                path: "src/main/kotlin/demo/Beta.kt".into(),
                line: 21,
                symbol: "demo.beta".into(),
            }],
        )
        .unwrap();

    let first = store.tests(None, 1).unwrap();
    assert_eq!(first.items[0].id, "demo.AlphaTest#works");
    let second = store.tests(first.next_cursor.as_deref(), 1).unwrap();
    assert_eq!(second.items[0].id, "demo.BetaTest#fails");
    assert_eq!(
        second.items[0].last_failure_id.as_deref(),
        Some("failure-1")
    );
    assert!(second.next_cursor.is_none());

    assert_eq!(
        store
            .failure("failure-1")
            .unwrap()
            .unwrap()
            .event_id
            .as_deref(),
        Some("evt-failure")
    );
    assert_eq!(
        store
            .coverage("src/main/kotlin/demo/Beta.kt", None, 100)
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        store
            .trace("demo.BetaTest#fails", None, 100, 64 * 1024)
            .unwrap()
            .items,
        vec![event.clone()]
    );
    assert_eq!(
        store.event("evt-failure").unwrap().unwrap().values["actual"],
        2
    );

    store
        .finish_run("run-queries", RunStatus::Passed, "rev-query", Utc::now())
        .unwrap();
    assert_eq!(
        store.latest_passed_revision().unwrap().as_deref(),
        Some("rev-query")
    );
}
