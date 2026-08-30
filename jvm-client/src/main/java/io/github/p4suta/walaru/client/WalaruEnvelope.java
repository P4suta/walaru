package io.github.p4suta.walaru.client;

import java.util.List;
import java.util.Map;

/** Versioned response envelope returned by every typed operation. */
public record WalaruEnvelope<T>(
        String schemaVersion,
        String workspaceId,
        String revision,
        String sessionId,
        String runId,
        WalaruStatus status,
        T data,
        List<Diagnostic> diagnostics,
        CapabilityManifest capabilities,
        List<NextAction> nextActions,
        Page page) {
    public record Diagnostic(String code, String severity, String message, Map<String, String> details) {}

    public record CapabilityManifest(
            String backend, String completeness, List<String> supported, Map<String, String> unavailable) {}

    public record NextAction(String title, List<String> argv) {}

    public record Page(String cursor, String nextCursor, int limit, int returned) {}
}
