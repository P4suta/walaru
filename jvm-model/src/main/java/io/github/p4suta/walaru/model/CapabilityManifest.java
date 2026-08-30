package io.github.p4suta.walaru.model;

import java.util.List;
import java.util.Map;

/** Explicitly separates captured replay scope from unavailable capabilities. */
public record CapabilityManifest(
        String backend,
        Completeness completeness,
        List<String> supported,
        Map<String, String> unavailable) {
    public CapabilityManifest {
        supported = List.copyOf(supported);
        unavailable = Map.copyOf(unavailable);
    }

    /** M1's exact single-test, single-thread, pure-JVM scope. */
    public static CapabilityManifest m1PureJvm() {
        return new CapabilityManifest(
                "jvm",
                Completeness.COMPLETE,
                List.of("singleTest", "singleThread", "pureJvm", "line", "call", "write"),
                Map.of(
                        "threads", "multiple application threads are not recorded",
                        "native", "JNI inputs are not recorded",
                        "io", "external I/O is not recorded",
                        "subprocess", "child processes are not recorded",
                        "checkpoint", "no CRaC/CRIU checkpoint adapter is configured"));
    }
}
