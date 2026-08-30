package io.github.p4suta.walaru.client;

/** Stable process outcomes shared by the CLI and typed client. */
public enum WalaruExit {
    SUCCESS(0),
    TEST_FAILURE(1),
    USAGE(2),
    INTERNAL_ERROR(3),
    INCOMPLETE_OR_STALE(4),
    UNKNOWN(-1);

    private final int code;

    WalaruExit(int code) {
        this.code = code;
    }

    public int code() {
        return code;
    }

    public static WalaruExit fromCode(int code) {
        for (WalaruExit value : values()) {
            if (value.code == code) return value;
        }
        return UNKNOWN;
    }
}
