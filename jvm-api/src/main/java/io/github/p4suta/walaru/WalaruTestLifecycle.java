package io.github.p4suta.walaru;

/**
 * Framework-adapter SPI for associating runtime events with a test.
 *
 * <p>Application code should use {@link Walaru}; JUnit Platform and TestNG adapters call this SPI.
 * Like the rest of the API, it is fail-open when the agent is absent.
 */
public final class WalaruTestLifecycle {
    private WalaruTestLifecycle() {}

    public static void started(String uniqueId, String publicName) {
        RuntimeBridge.testStarted(uniqueId, publicName);
    }

    public static void finished(String uniqueId, String status, Throwable failure) {
        RuntimeBridge.testFinished(uniqueId, status, failure);
    }
}
