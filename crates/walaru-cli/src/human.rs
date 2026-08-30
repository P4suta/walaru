//! Command-aware human output built from the stable structured envelope.

use std::io::{self, Write};

use serde_json::Value;
use walaru_core::protocol::{Envelope, Status};

pub(crate) fn render(
    output: &mut impl Write,
    command: &str,
    envelope: &Envelope,
) -> io::Result<()> {
    writeln!(
        output,
        "Walaru {} · {}",
        env!("CARGO_PKG_VERSION"),
        status(envelope.status)
    )?;
    match command {
        "status" => render_status(output, envelope)?,
        "stop" => writeln!(output, "Daemon shutdown requested.")?,
        "doctor" => render_doctor(output, envelope)?,
        "verify" => render_verify(output, envelope)?,
        "explain" => render_explain(output, envelope)?,
        "tests" => render_tests(output, envelope)?,
        "failure" => render_failure(output, envelope)?,
        "impact" => render_impact(output, envelope)?,
        "coverage" => render_coverage(output, envelope)?,
        "trace" => render_trace(output, envelope)?,
        "values" => render_values(output, envelope)?,
        "record" => render_record(output, envelope)?,
        "replay" | "reverse" => render_replay(output, envelope)?,
        _ => render_fallback(output, &envelope.data)?,
    }
    render_diagnostics(output, envelope)?;
    render_actions(output, envelope)
}

fn render_status(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    writeln!(
        output,
        "Daemon:   {}",
        yes_no(bool_at(&envelope.data, "/running"))
    )?;
    writeln!(output, "Revision: {}", envelope.revision)?;
    writeln!(output, "Workspace: {}", envelope.workspace_id)?;
    if let Some(pid) = envelope.data.get("pid").and_then(Value::as_u64) {
        writeln!(output, "Process:  {pid}")?;
    }
    writeln!(
        output,
        "Replay:   {} ({})",
        envelope.capabilities.backend,
        completeness(envelope)
    )
}

fn render_doctor(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    writeln!(
        output,
        "Ready: {}",
        yes_no(bool_at(&envelope.data, "/ready"))
    )?;
    writeln!(output)?;
    writeln!(output, "CHECK             RESULT  DETAILS")?;
    let os = string_at(&envelope.data, "/platform/os");
    let arch = string_at(&envelope.data, "/platform/arch");
    row(
        output,
        "Platform",
        bool_at(&envelope.data, "/platform/supported"),
        &format!("{os}/{arch}"),
    )?;
    row(
        output,
        "JDK 21+",
        envelope
            .data
            .pointer("/java/major")
            .and_then(Value::as_u64)
            .is_some_and(|major| major >= 21),
        string_at(&envelope.data, "/java/raw"),
    )?;
    row(
        output,
        "Build tool",
        bool_at(&envelope.data, "/buildTool/ready"),
        string_at(&envelope.data, "/buildTool/kind"),
    )?;
    row(
        output,
        "Runtime artifacts",
        bool_at(&envelope.data, "/runtimeArtifacts/present"),
        "agent and adapter",
    )?;
    row(
        output,
        "Exact Linux replay",
        bool_at(&envelope.data, "/linuxReplay/rr"),
        "optional rr backend",
    )
}

fn render_verify(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    writeln!(
        output,
        "Verification: {}",
        string_at(&envelope.data, "/status")
    )?;
    if let Some(run_id) = &envelope.run_id {
        writeln!(output, "Run:          {run_id}")?;
    }
    let tests = envelope
        .data
        .get("tests")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    writeln!(output, "Tests:        {tests}")?;
    if let Some(events) = envelope.data.get("events").and_then(Value::as_u64) {
        writeln!(output, "Events:       {events}")?;
    }
    if bool_at(&envelope.data, "/requeued") {
        writeln!(output, "Revision changed once; verification was requeued.")?;
    }
    Ok(())
}

fn render_explain(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    let verification = envelope.data.get("verification").unwrap_or(&Value::Null);
    writeln!(
        output,
        "Verification: {}",
        string_at(verification, "/status")
    )?;
    if let Some(run_id) = &envelope.run_id {
        writeln!(output, "Run:          {run_id}")?;
    }
    let explanations = envelope
        .data
        .get("explanations")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if explanations.is_empty() {
        if let Some(build) = envelope
            .data
            .get("buildFailure")
            .filter(|value| !value.is_null())
        {
            writeln!(output, "{}", string_at(build, "/summary"))?;
            if let Some(log) = build.get("logFile").and_then(Value::as_str) {
                writeln!(output, "Worker log:    {log}")?;
            }
            if let Some(suggestions) = build.get("suggestions").and_then(Value::as_array) {
                writeln!(output, "Next steps:")?;
                for suggestion in suggestions {
                    writeln!(output, "  - {}", suggestion.as_str().unwrap_or("-"))?;
                }
            }
            return Ok(());
        }
        return writeln!(output, "No failed test produced explainable evidence.");
    }
    for explanation in explanations {
        let failure = explanation.get("failure").unwrap_or(&Value::Null);
        let analysis = explanation.get("analysis").unwrap_or(&Value::Null);
        let recording = explanation.get("recording").unwrap_or(&Value::Null);
        writeln!(output)?;
        writeln!(output, "{}", string_at(failure, "/testId"))?;
        writeln!(output, "  {}", string_at(analysis, "/summary"))?;
        writeln!(output, "  Likely: {}", string_at(analysis, "/likelyCause"))?;
        if let Some(focus) = analysis.get("focus").filter(|value| !value.is_null()) {
            writeln!(output, "  Focus:  {}", location(Some(focus)))?;
        }
        if let Some(recording_id) = recording.get("id").and_then(Value::as_str) {
            writeln!(
                output,
                "  Full recording: {recording_id} ({} events, {})",
                recording
                    .get("events")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                string_at(recording, "/capabilities/completeness")
            )?;
        } else if let Some(error) = recording.get("error").and_then(Value::as_str) {
            writeln!(output, "  Full recording unavailable: {error}")?;
        }
        if let Some(evidence) = analysis.get("evidence").and_then(Value::as_array) {
            for item in evidence.iter().take(5) {
                let value = item
                    .get("value")
                    .map_or_else(|| "-".into(), Value::to_string);
                writeln!(
                    output,
                    "    {} = {}",
                    string_at(item, "/label"),
                    clipped(&value, 100)
                )?;
            }
        }
    }
    let omitted = envelope
        .data
        .get("omittedFailures")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if omitted > 0 {
        writeln!(output, "\n{omitted} additional failures were not recorded.")?;
    }
    Ok(())
}

fn render_tests(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    let tests = envelope
        .data
        .get("tests")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    writeln!(output, "Tests: {}", tests.len())?;
    writeln!(output)?;
    writeln!(
        output,
        "STATUS    MODULE           TEST                                      FAILURE"
    )?;
    for test in tests {
        writeln!(
            output,
            "{:<9} {:<16} {:<41} {}",
            clipped(string_at(test, "/lastStatus"), 8),
            clipped(string_at(test, "/module"), 15),
            clipped(string_at(test, "/id"), 40),
            clipped(string_at(test, "/lastFailureId"), 24),
        )?;
    }
    render_page(output, envelope)
}

fn render_failure(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    let Some(failure) = envelope
        .data
        .get("failure")
        .filter(|value| !value.is_null())
    else {
        return writeln!(output, "No failure found.");
    };
    writeln!(output, "Failure: {}", string_at(failure, "/id"))?;
    writeln!(output, "Test:    {}", string_at(failure, "/testId"))?;
    writeln!(output, "Type:    {}", string_at(failure, "/exceptionType"))?;
    writeln!(output, "Message: {}", string_at(failure, "/message"))?;
    if let Some(event_id) = failure.get("eventId").and_then(Value::as_str) {
        writeln!(output, "Event:   {event_id}")?;
    }
    if let Some(analysis) = envelope
        .data
        .get("analysis")
        .filter(|value| !value.is_null())
    {
        writeln!(output)?;
        writeln!(output, "Why this probably failed")?;
        writeln!(output, "  {}", string_at(analysis, "/summary"))?;
        writeln!(
            output,
            "  Evidence: {}",
            string_at(analysis, "/likelyCause")
        )?;
        if let Some(focus) = analysis.get("focus").filter(|value| !value.is_null()) {
            writeln!(output, "  Focus:    {}", location(Some(focus)))?;
        }
        if let Some(evidence) = analysis.get("evidence").and_then(Value::as_array) {
            writeln!(output)?;
            writeln!(output, "Relevant state:")?;
            for item in evidence.iter().take(7) {
                let rendered = item
                    .get("value")
                    .map_or_else(|| "-".into(), Value::to_string);
                writeln!(
                    output,
                    "  {} = {}",
                    string_at(item, "/label"),
                    clipped(&rendered, 120)
                )?;
            }
        }
        if let Some(suggestions) = analysis.get("suggestions").and_then(Value::as_array) {
            writeln!(output)?;
            writeln!(output, "Try next:")?;
            for suggestion in suggestions.iter().filter_map(Value::as_str).take(4) {
                writeln!(output, "  - {suggestion}")?;
            }
        }
    }
    if let Some(frames) = failure.get("frames").and_then(Value::as_array) {
        writeln!(output)?;
        writeln!(output, "Top stack frames:")?;
        for frame in frames.iter().take(8) {
            writeln!(output, "  {}", frame.as_str().unwrap_or("?"))?;
        }
        if frames.len() > 8 {
            writeln!(output, "  … {} more", frames.len() - 8)?;
        }
    }
    Ok(())
}

fn render_impact(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    writeln!(
        output,
        "Subject:   {}",
        string_at(&envelope.data, "/subject")
    )?;
    writeln!(
        output,
        "Selection: {}",
        string_at(&envelope.data, "/selection")
    )?;
    if let Some(reason) = envelope.data.get("reason").and_then(Value::as_str) {
        writeln!(output, "Reason:    {reason}")?;
    }
    if let Some(tests) = envelope.data.get("tests").and_then(Value::as_array) {
        for test in tests {
            writeln!(output, "  {}", test.as_str().unwrap_or("?"))?;
        }
    }
    Ok(())
}

fn render_coverage(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    let coverage = envelope
        .data
        .get("coverage")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    writeln!(output, "Coverage entries: {}", coverage.len())?;
    writeln!(
        output,
        "PATH:LINE                                SYMBOL                         TEST"
    )?;
    for item in coverage {
        let location = format!(
            "{}:{}",
            string_at(item, "/path"),
            item.get("line").and_then(Value::as_u64).unwrap_or_default()
        );
        writeln!(
            output,
            "{:<40} {:<30} {}",
            clipped(&location, 39),
            clipped(string_at(item, "/symbol"), 29),
            string_at(item, "/testId")
        )?;
    }
    render_page(output, envelope)
}

fn render_trace(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    let events = envelope
        .data
        .get("events")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    writeln!(output, "Trace events: {}", events.len())?;
    writeln!(
        output,
        "SEQ      KIND          SOURCE                                  THREAD"
    )?;
    for event in events {
        let source = location(event.get("location"));
        writeln!(
            output,
            "{:<8} {:<13} {:<39} {}",
            event
                .get("sequence")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            clipped(string_at(event, "/kind"), 12),
            clipped(&source, 38),
            event
                .get("threadId")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        )?;
    }
    render_page(output, envelope)
}

fn render_values(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    writeln!(output, "Event: {}", string_at(&envelope.data, "/eventId"))?;
    writeln!(
        output,
        "Source: {}",
        location(envelope.data.get("location"))
    )?;
    writeln!(output, "Values:")?;
    render_json(
        output,
        envelope.data.get("values").unwrap_or(&Value::Null),
        2,
    )?;
    if let Some(observations) = envelope
        .data
        .get("observations")
        .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
    {
        writeln!(output, "Observations:")?;
        render_json(output, observations, 2)?;
    }
    Ok(())
}

fn render_record(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    writeln!(
        output,
        "Recording: {}",
        string_at(&envelope.data, "/recordingId")
    )?;
    writeln!(
        output,
        "Test:      {}",
        string_at(&envelope.data, "/testId")
    )?;
    writeln!(
        output,
        "Events:    {}",
        display_at(&envelope.data, "/events")
    )?;
    writeln!(output, "Replay:    {}", completeness(envelope))
}

fn render_replay(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    writeln!(
        output,
        "Recording: {}",
        string_at(&envelope.data, "/recordingId")
    )?;
    writeln!(
        output,
        "Verified:  {}",
        yes_no(bool_at(&envelope.data, "/verified"))
    )?;
    if let Some(event) = envelope.data.get("event") {
        writeln!(output, "Event:     {}", string_at(event, "/id"))?;
        writeln!(output, "Position:  {}", location(event.get("location")))?;
        writeln!(output, "Values:")?;
        render_json(output, event.get("values").unwrap_or(&Value::Null), 2)?;
    }
    Ok(())
}

fn render_fallback(output: &mut impl Write, data: &Value) -> io::Result<()> {
    render_json(output, data, 0)
}

fn render_diagnostics(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    if envelope.diagnostics.is_empty() {
        return Ok(());
    }
    writeln!(output)?;
    writeln!(output, "Diagnostics:")?;
    for item in &envelope.diagnostics {
        writeln!(
            output,
            "  [{}] {}: {}",
            item.severity.to_uppercase(),
            item.code,
            item.message
        )?;
    }
    Ok(())
}

fn render_actions(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    if envelope.next_actions.is_empty() {
        return Ok(());
    }
    writeln!(output)?;
    writeln!(output, "Next actions:")?;
    for action in &envelope.next_actions {
        writeln!(output, "  {}", action.title)?;
        writeln!(output, "    argv: {}", action.argv.join(" | "))?;
    }
    Ok(())
}

fn render_page(output: &mut impl Write, envelope: &Envelope) -> io::Result<()> {
    if let Some(page) = &envelope.page
        && let Some(cursor) = &page.next_cursor
    {
        writeln!(output)?;
        writeln!(output, "More results: --cursor {cursor}")?;
    }
    Ok(())
}

fn render_json(output: &mut impl Write, value: &Value, indent: usize) -> io::Result<()> {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".into());
    let padding = " ".repeat(indent);
    for line in rendered.lines() {
        writeln!(output, "{padding}{line}")?;
    }
    Ok(())
}

fn row(output: &mut impl Write, name: &str, result: bool, details: &str) -> io::Result<()> {
    writeln!(
        output,
        "{:<17} {:<7} {}",
        name,
        if result { "ok" } else { "missing" },
        details
    )
}

fn string_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("-")
}

fn display_at(value: &Value, pointer: &str) -> String {
    value.pointer(pointer).map_or_else(
        || "-".into(),
        |item| {
            item.as_str()
                .map_or_else(|| item.to_string(), str::to_owned)
        },
    )
}

fn bool_at(value: &Value, pointer: &str) -> bool {
    value.pointer(pointer).and_then(Value::as_bool) == Some(true)
}

fn status(value: Status) -> &'static str {
    match value {
        Status::Ok => "ok",
        Status::Failure => "failure",
        Status::Partial => "partial",
        Status::Stale => "stale",
        Status::Unsupported => "unsupported",
        Status::Error => "error",
    }
}

fn completeness(envelope: &Envelope) -> &'static str {
    match envelope.capabilities.completeness {
        walaru_core::protocol::Completeness::Complete => "complete",
        walaru_core::protocol::Completeness::Partial => "partial",
        walaru_core::protocol::Completeness::Unsupported => "unsupported",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn location(value: Option<&Value>) -> String {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return "-".into();
    };
    format!(
        "{}:{}",
        string_at(value, "/path"),
        value
            .get("line")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    )
}

fn clipped(value: &str, width: usize) -> String {
    let mut chars = value.chars();
    let clipped = chars.by_ref().take(width).collect::<String>();
    if chars.next().is_some() && width > 1 {
        format!("{}…", clipped.chars().take(width - 1).collect::<String>())
    } else {
        clipped
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use walaru_core::protocol::{CapabilityManifest, Completeness, Envelope, Status};

    use super::render;

    fn envelope(data: serde_json::Value) -> Envelope {
        Envelope {
            schema_version: "1".into(),
            workspace_id: "ws-test".into(),
            revision: "rev-test".into(),
            session_id: "session-test".into(),
            run_id: None,
            status: Status::Ok,
            data,
            diagnostics: Vec::new(),
            capabilities: CapabilityManifest {
                backend: "none".into(),
                completeness: Completeness::Unsupported,
                supported: Vec::new(),
                unavailable: BTreeMap::default(),
            },
            next_actions: Vec::new(),
            page: None,
        }
    }

    #[test]
    fn tests_renderer_is_a_table_and_surfaces_failure_ids() {
        let value = envelope(json!({"tests": [{
            "id": "demo.ExampleTest#fails",
            "displayName": "fails",
            "module": ":app",
            "lastStatus": "failed",
            "lastFailureId": "failure-7"
        }]}));
        let mut output = Vec::new();
        render(&mut output, "tests", &value).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("STATUS"));
        assert!(output.contains("demo.ExampleTest#fails"));
        assert!(output.contains("failure-7"));
        assert!(!output.contains("\"tests\""));
    }

    #[test]
    fn failure_renderer_includes_diagnosis_without_json_dump() {
        let value = envelope(json!({"failure": {
            "id": "failure-1",
            "testId": "demo.ExampleTest#fails",
            "exceptionType": "java.lang.AssertionError",
            "message": "expected 1",
            "eventId": "evt-1",
            "frames": ["ExampleTest.kt:9"]
        }, "analysis": {
            "summary": "Assertion failed: expected 1, observed 2.",
            "likelyCause": "Captured `actual` with value 2 immediately preceded the failure.",
            "focus": {"path": "src/test/kotlin/ExampleTest.kt", "line": 9},
            "evidence": [{"label": "Captured `actual`", "value": 2}],
            "suggestions": ["Inspect the focused source line."]
        }}));
        let mut output = Vec::new();
        render(&mut output, "failure", &value).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("java.lang.AssertionError"));
        assert!(output.contains("expected 1"));
        assert!(output.contains("ExampleTest.kt:9"));
        assert!(output.contains("Why this probably failed"));
        assert!(output.contains("Captured `actual`"));
    }
}
