package io.github.p4suta.walaru.client;

/** A parsed envelope plus its authoritative CLI exit classification. */
public record WalaruResult<T>(WalaruExit exit, int exitCode, WalaruEnvelope<T> envelope) {
    public boolean succeeded() {
        return exit == WalaruExit.SUCCESS;
    }

    public boolean hasUsableData() {
        return exit == WalaruExit.SUCCESS
                || exit == WalaruExit.TEST_FAILURE
                || exit == WalaruExit.INCOMPLETE_OR_STALE;
    }
}
