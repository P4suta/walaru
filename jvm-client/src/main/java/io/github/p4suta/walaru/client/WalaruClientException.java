package io.github.p4suta.walaru.client;

/** Process, timeout, size-bound, or schema failure before a typed result could be returned. */
public final class WalaruClientException extends RuntimeException {
    private static final long serialVersionUID = 1L;
    private final Integer exitCode;

    WalaruClientException(String message) {
        this(message, null, null);
    }

    WalaruClientException(String message, Throwable cause) {
        this(message, null, cause);
    }

    WalaruClientException(String message, Integer exitCode) {
        this(message, exitCode, null);
    }

    private WalaruClientException(String message, Integer exitCode, Throwable cause) {
        super(message, cause);
        this.exitCode = exitCode;
    }

    public Integer exitCode() {
        return exitCode;
    }
}
