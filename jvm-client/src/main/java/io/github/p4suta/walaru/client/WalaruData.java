package io.github.p4suta.walaru.client;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/** Typed command payloads. JSON values remain trees because captured user types are open-ended. */
public final class WalaruData {
    private WalaruData() {}

    public record Status(
            boolean running,
            long pid,
            String version,
            String stateDirectory,
            String database,
            String socket) {}

    public record Tests(List<TestCase> tests) {}

    public record TestCase(
            String id, String displayName, String module, String lastStatus, String lastFailureId) {}

    public record FailureResponse(Failure failure, FailureAnalysis analysis) {}

    public record Failure(
            String id,
            String runId,
            String testId,
            String exceptionType,
            String message,
            String eventId,
            List<String> frames) {}

    public record FailureAnalysis(
            String summary,
            String likelyCause,
            Location focus,
            List<Evidence> evidence,
            List<String> suggestions) {}

    public record Evidence(String kind, String label, String eventId, Location location, JsonNode value) {}

    public record Verify(
            String runId,
            String revision,
            String status,
            int exitCode,
            List<String> tests,
            List<String> failures,
            int events,
            boolean cached,
            String selection,
            List<String> selectedTests,
            boolean requeued) {}

    public record Explain(
            Verify verification,
            List<Explanation> explanations,
            int omittedFailures,
            BuildFailure buildFailure) {}

    public record BuildFailure(String summary, String logFile, List<String> suggestions) {}

    public record Explanation(
            Failure failure, FailureAnalysis analysis, FullRecording recording) {}

    public record FullRecording(
            String id,
            String testId,
            String revision,
            int events,
            WalaruEnvelope.CapabilityManifest capabilities,
            String error) {}

    public record Trace(List<Event> events) {}

    public record Values(
            String eventId, Location location, JsonNode values, JsonNode observations, String stateHash) {}

    public record Event(
            String id,
            long sequence,
            long threadId,
            String threadKey,
            boolean virtualThread,
            boolean coroutine,
            String kind,
            Location location,
            JsonNode values,
            JsonNode observations,
            String stateHash,
            long outputIndex) {}

    public record Location(String path, int line, int column, String symbol) {}

    public record Impact(String subject, String selection, List<String> tests, String reason) {}

    public record Coverage(List<CoverageEntry> coverage) {}

    public record CoverageEntry(String testId, String path, int line, String symbol) {}

    public record Recording(String recordingId, String testId, String revision, int events) {}

    public record Replay(
            String recordingId,
            String replayRunId,
            String backend,
            Event event,
            boolean verified) {}
}
