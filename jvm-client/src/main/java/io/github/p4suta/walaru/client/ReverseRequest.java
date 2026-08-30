package io.github.p4suta.walaru.client;

import java.util.Objects;

/** One reverse-navigation operation. */
public record ReverseRequest(
        String recordingId, String fromEventId, Step step, String until, String watch) {
    public enum Step {
        LINE,
        CALL,
        WRITE
    }

    public ReverseRequest {
        Objects.requireNonNull(recordingId, "recordingId");
        Objects.requireNonNull(fromEventId, "fromEventId");
        if ((step == null) == (until == null || until.isBlank())) {
            throw new IllegalArgumentException("provide exactly one of step or until");
        }
        if (watch != null && step != Step.WRITE) {
            throw new IllegalArgumentException("watch requires the WRITE step");
        }
    }

    public static ReverseRequest step(
            String recordingId, String fromEventId, Step step) {
        return new ReverseRequest(recordingId, fromEventId, step, null, null);
    }

    public static ReverseRequest watch(
            String recordingId, String fromEventId, String watch) {
        return new ReverseRequest(recordingId, fromEventId, Step.WRITE, null, watch);
    }

    public static ReverseRequest until(
            String recordingId, String fromEventId, String sourceAndLine) {
        return new ReverseRequest(recordingId, fromEventId, null, sourceAndLine, null);
    }
}
