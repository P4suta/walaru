//! Deterministic, local failure explanation from already-redacted trace evidence.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::replay::{Event, EventKind, SourceLocation};
use crate::store::FailureRecord;

/// Bounded explanation suitable for CLI, editor, SDK, and generated reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureAnalysis {
    /// One-line failure classification.
    pub summary: String,
    /// Evidence-backed likely cause; this is deterministic analysis, not an AI claim.
    pub likely_cause: String,
    /// Best source position inferred from the trace.
    pub focus: Option<SourceLocation>,
    /// Most useful recent values, checkpoints, spans, and writes.
    pub evidence: Vec<FailureEvidence>,
    /// Small ordered list of concrete follow-ups.
    pub suggestions: Vec<String>,
}

/// One safely captured fact used by [`FailureAnalysis`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureEvidence {
    /// Stable evidence category.
    pub kind: String,
    /// Human-readable, bounded label.
    pub label: String,
    /// Trace event that supplied the evidence, when applicable.
    pub event_id: Option<String>,
    /// Source position attached to that event.
    pub location: Option<SourceLocation>,
    /// Already-redacted structured value.
    pub value: Value,
}

/// Produces a useful explanation without network access or executing user code.
#[must_use]
pub fn analyze_failure(failure: &FailureRecord, events: &[Event]) -> FailureAnalysis {
    let mismatch = assertion_mismatch(&failure.message);
    let mut evidence = Vec::new();
    if let Some((expected, actual)) = &mismatch {
        evidence.push(FailureEvidence {
            kind: "assertionMismatch".into(),
            label: "Expected and observed values differ".into(),
            event_id: failure.event_id.clone(),
            location: None,
            value: json!({"expected": expected, "actual": actual}),
        });
    }

    for event in events.iter().rev() {
        if evidence.len() >= 7 {
            break;
        }
        let Some((kind, label, value)) = evidence_from_event(event) else {
            continue;
        };
        evidence.push(FailureEvidence {
            kind: kind.into(),
            label,
            event_id: Some(event.id.clone()),
            location: event.location.clone(),
            value,
        });
    }

    let focus = evidence
        .iter()
        .filter(|item| actionable(item))
        .find_map(|item| item.location.clone())
        .or_else(|| evidence.iter().find_map(|item| item.location.clone()))
        .or_else(|| events.iter().rev().find_map(|event| event.location.clone()));
    let type_name = simple_type(&failure.exception_type);
    let (summary, likely_cause, suggestions) = if let Some((expected, actual)) = mismatch {
        (
            format!(
                "Assertion failed: expected {}, observed {}.",
                clipped(&expected, 96),
                clipped(&actual, 96)
            ),
            explicit_or(
                &evidence,
                format!("The assertion compared different values ({expected} versus {actual})."),
            ),
            vec![
                "Inspect the focused source line and the latest named captures before the assertion."
                    .into(),
                "Record this test in full when the fast trace does not include the state transition."
                    .into(),
            ],
        )
    } else if failure.exception_type.contains("NullPointerException") {
        (
            "A null value was dereferenced.".into(),
            explicit_or(
                &evidence,
                "A value required by the failing code was null.".into(),
            ),
            vec![
                "Inspect the latest capture or write feeding the focused line.".into(),
                "Add Walaru.capture at the nullable boundary if the producer is not visible."
                    .into(),
            ],
        )
    } else if failure.exception_type.contains("IndexOutOfBounds") {
        (
            "A collection or array index was outside its valid range.".into(),
            explicit_or(&evidence, clipped(&failure.message, 256)),
            vec![
                "Compare the captured index with the collection length at the focused line.".into(),
                "Add a checkpoint at the loop or partition boundary when those values are absent."
                    .into(),
            ],
        )
    } else {
        (
            format!("{type_name}: {}", clipped(&failure.message, 256)),
            explicit_or(&evidence, clipped(&failure.message, 256)),
            vec![
                "Inspect the focused source line and recent trace evidence.".into(),
                "Record this test in full for ordered reads, writes, and safely captured values."
                    .into(),
            ],
        )
    };

    FailureAnalysis {
        summary,
        likely_cause,
        focus,
        evidence,
        suggestions,
    }
}

fn evidence_from_event(event: &Event) -> Option<(&'static str, String, Value)> {
    let name = event.values.get("name").and_then(Value::as_str);
    match event.kind {
        EventKind::Capture => Some((
            "capture",
            format!("Captured `{}`", name.unwrap_or("value")),
            event.values.get("value").cloned().unwrap_or(Value::Null),
        )),
        EventKind::Checkpoint => Some((
            "checkpoint",
            format!("Reached checkpoint `{}`", name.unwrap_or("unnamed")),
            event.values.get("value").cloned().unwrap_or(Value::Null),
        )),
        EventKind::Note => Some((
            "note",
            format!("Note `{}`", name.unwrap_or("unnamed")),
            event.values.get("message").cloned().unwrap_or(Value::Null),
        )),
        EventKind::SpanValue => Some((
            "spanValue",
            format!("Span captured `{}`", name.unwrap_or("value")),
            event.values.get("value").cloned().unwrap_or(Value::Null),
        )),
        EventKind::SpanEnd
            if event.values.get("status").and_then(Value::as_str) == Some("failed") =>
        {
            Some((
                "failedSpan",
                format!("Span `{}` failed", name.unwrap_or("unnamed")),
                event.values.clone(),
            ))
        }
        EventKind::Write => {
            let target = match (
                event.values.get("field").and_then(Value::as_str),
                event.values.get("index").and_then(Value::as_i64),
            ) {
                (Some(field), _) => format!("field `{field}`"),
                (_, Some(index)) => format!("array index {index}"),
                _ => "state".into(),
            };
            Some((
                "write",
                format!("Last observed write to {target}"),
                event.values.clone(),
            ))
        }
        _ => None,
    }
}

fn explicit_or(evidence: &[FailureEvidence], fallback: String) -> String {
    let Some(item) = evidence.iter().find(|item| {
        actionable(item)
            && matches!(
                item.kind.as_str(),
                "capture" | "checkpoint" | "spanValue" | "failedSpan" | "write"
            )
    }) else {
        return fallback;
    };
    let value = clipped(&item.value.to_string(), 160);
    if value == "null" {
        format!("{} immediately preceded the failure.", item.label)
    } else {
        format!(
            "{} with value {value} immediately preceded the failure.",
            item.label
        )
    }
}

fn actionable(item: &FailureEvidence) -> bool {
    !matches!(item.value.as_str(), Some(value) if value.starts_with("<redacted"))
}

fn assertion_mismatch(message: &str) -> Option<(String, String)> {
    let expected_start = find_ascii_case_insensitive(message, "expected", 0)? + "expected".len();
    let (separator, marker) = ["but was", "but found"]
        .into_iter()
        .filter_map(|marker| {
            find_ascii_case_insensitive(message, marker, expected_start)
                .map(|position| (position, marker))
        })
        .min_by_key(|(position, _)| *position)?;
    let actual_start = separator + marker.len();
    let expected = trim_assertion_value(&message[expected_start..separator]);
    let actual = trim_assertion_value(&message[actual_start..]);
    (!expected.is_empty() && !actual.is_empty()).then_some((expected, actual))
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str, start: usize) -> Option<usize> {
    haystack
        .as_bytes()
        .get(start..)?
        .windows(needle.len())
        .position(|candidate| candidate.eq_ignore_ascii_case(needle.as_bytes()))
        .map(|position| start + position)
}

fn trim_assertion_value(value: &str) -> String {
    value
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    ':' | '=' | '<' | '[' | '(' | '.' | '>' | ']' | ')'
                )
        })
        .to_owned()
}

fn simple_type(value: &str) -> &str {
    value.rsplit('.').next().unwrap_or(value)
}

fn clipped(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.into();
    }
    value.chars().take(maximum).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventKind, values: Value) -> Event {
        Event {
            id: "evt-1".into(),
            sequence: 1,
            thread_id: 1,
            thread_key: "platform:test".into(),
            virtual_thread: false,
            coroutine: false,
            logical_stack: Vec::new(),
            kind,
            location: Some(SourceLocation {
                path: "src/main/java/demo/Search.java".into(),
                line: 18,
                column: 1,
                symbol: "demo.Search#find".into(),
            }),
            values,
            observations: json!({}),
            state_hash: "state".into(),
            output_index: 0,
        }
    }

    fn failure(message: &str) -> FailureRecord {
        FailureRecord {
            id: "failure-1".into(),
            run_id: "run-1".into(),
            test_id: "demo.SearchTest#finds".into(),
            exception_type: "org.opentest4j.AssertionFailedError".into(),
            message: message.into(),
            event_id: Some("evt-finish".into()),
            frames: Vec::new(),
        }
    }

    #[test]
    fn assertion_and_explicit_capture_become_concrete_bounded_evidence() {
        let analysis = analyze_failure(
            &failure("expected: <4> but was: <5>"),
            &[event(
                EventKind::Capture,
                json!({"name": "low", "value": 5}),
            )],
        );

        assert!(analysis.summary.contains("expected 4, observed 5"));
        assert!(analysis.likely_cause.contains("Captured `low`"));
        assert_eq!(analysis.focus.unwrap().line, 18);
        assert_eq!(analysis.evidence[0].kind, "assertionMismatch");
        assert_eq!(analysis.evidence[1].value, json!(5));
    }

    #[test]
    fn redacted_values_remain_redacted_in_the_explanation() {
        let analysis = analyze_failure(
            &failure("boom"),
            &[event(
                EventKind::Capture,
                json!({"name": "apiToken", "value": "<redacted>"}),
            )],
        );

        assert!(
            serde_json::to_string(&analysis)
                .unwrap()
                .contains("<redacted>")
        );
    }

    #[test]
    fn assertion_parser_keeps_original_indices_with_unicode_prefixes() {
        let analysis = analyze_failure(&failure("İstanbul EXPECTED: <4> but found: <5>"), &[]);

        assert_eq!(
            analysis.summary,
            "Assertion failed: expected 4, observed 5."
        );
    }
}
