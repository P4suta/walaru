package io.github.p4suta.walaru;

import java.util.Objects;
import java.util.concurrent.Callable;
import java.util.function.Supplier;

/**
 * Stable, zero-dependency instrumentation API for Java and Kotlin code.
 *
 * <p>Every operation is fail-open. When the Walaru agent is absent, disabled, or outside a test,
 * values are returned unchanged and spans become no-ops. Application behavior must never depend on
 * Walaru being installed.
 */
public final class Walaru {
    private Walaru() {}

    /** Returns whether the current thread is inside a test captured by Walaru. */
    public static boolean active() {
        return RuntimeBridge.active();
    }

    /** Captures the current test context for callbacks or pre-existing executor threads. */
    public static WalaruContext context() {
        return WalaruContext.capture();
    }

    /**
     * Records a safely bounded snapshot under {@code name} and returns {@code value} unchanged.
     * Secret-looking names and values are redacted by the runtime.
     */
    public static <T> T capture(String name, T value) {
        RuntimeBridge.capture(normalize(name), value, false, RuntimeBridge.caller());
        return value;
    }

    /**
     * Records only that a named secret existed and returns {@code value} unchanged. The value is
     * never handed to the recording backend.
     */
    public static <T> T captureRedacted(String name, T value) {
        RuntimeBridge.capture(normalize(name), null, true, RuntimeBridge.caller());
        return value;
    }

    /**
     * Computes and records diagnostic state only while Walaru is actively observing a test.
     * Non-fatal failures from the diagnostic supplier are ignored, so adding instrumentation does
     * not change application behavior. Serious {@link Error} signals other than assertion failures
     * are never hidden.
     */
    public static void captureLazy(String name, Supplier<?> diagnosticValue) {
        if (!RuntimeBridge.active() || diagnosticValue == null) return;
        try {
            capture(name, diagnosticValue.get());
        } catch (Throwable failure) {
            if (failure instanceof Error error && !(error instanceof AssertionError)) throw error;
            // Diagnostic computation is observational and deliberately fail-open.
        }
    }

    /** Adds a named point to the trace. */
    public static void checkpoint(String name) {
        RuntimeBridge.checkpoint(normalize(name), null, false, RuntimeBridge.caller());
    }

    /** Adds a named point and safe value snapshot to the trace, returning the value unchanged. */
    public static <T> T checkpoint(String name, T value) {
        RuntimeBridge.checkpoint(normalize(name), value, true, RuntimeBridge.caller());
        return value;
    }

    /** Adds a short, safely captured annotation to the trace. */
    public static void note(String name, String message) {
        RuntimeBridge.note(normalize(name), message, RuntimeBridge.caller());
    }

    /** Adds an unnamed short annotation to the trace. */
    public static void note(String message) {
        note("note", message);
    }

    /** Starts a source-linked span suitable for try-with-resources or Kotlin {@code use}. */
    public static WalaruSpan span(String name) {
        if (!RuntimeBridge.active()) return WalaruSpan.noop();
        RuntimeBridge.Caller caller = RuntimeBridge.caller();
        String id = RuntimeBridge.nextSpanId();
        RuntimeBridge.spanStarted(id, normalize(name), caller);
        return WalaruSpan.started(id, normalize(name), caller);
    }

    /** Runs an action inside a span and records whether it completed or threw. */
    public static void run(String name, Runnable action) {
        Objects.requireNonNull(action, "action");
        try (WalaruSpan span = span(name)) {
            try {
                action.run();
            } catch (RuntimeException | Error failure) {
                span.failed(failure);
                throw failure;
            }
        }
    }

    /** Calls an action inside a span and records whether it completed or threw. */
    public static <T> T call(String name, Callable<T> action) throws Exception {
        Objects.requireNonNull(action, "action");
        try (WalaruSpan span = span(name)) {
            try {
                return action.call();
            } catch (Exception | Error failure) {
                span.failed(failure);
                throw failure;
            }
        }
    }

    private static String normalize(String name) {
        if (name == null || name.isBlank()) return "unnamed";
        String stripped = name.strip();
        return stripped.length() <= 256 ? stripped : stripped.substring(0, 256);
    }
}
