package io.github.p4suta.walaru.client;

import com.fasterxml.jackson.annotation.JsonEnumDefaultValue;

/** Status inside a Walaru envelope, independent from the process exit code. */
public enum WalaruStatus {
    OK,
    FAILURE,
    PARTIAL,
    STALE,
    UNSUPPORTED,
    ERROR,
    @JsonEnumDefaultValue UNKNOWN
}
