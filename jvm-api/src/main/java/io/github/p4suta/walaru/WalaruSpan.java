package io.github.p4suta.walaru;

import java.util.concurrent.atomic.AtomicBoolean;

/** A fail-open trace span. Instances are created by {@link Walaru#span(String)}. */
public final class WalaruSpan implements AutoCloseable {
    private static final WalaruSpan NOOP = new WalaruSpan(null, null, null, 0L);

    private final String id;
    private final String name;
    private final RuntimeBridge.Caller caller;
    private final long startedNanos;
    private final AtomicBoolean closed = new AtomicBoolean();
    private volatile Throwable failure;

    private WalaruSpan(String id, String name, RuntimeBridge.Caller caller, long startedNanos) {
        this.id = id;
        this.name = name;
        this.caller = caller;
        this.startedNanos = startedNanos;
    }

    static WalaruSpan noop() {
        return NOOP;
    }

    static WalaruSpan started(String id, String name, RuntimeBridge.Caller caller) {
        return new WalaruSpan(id, name, caller, System.nanoTime());
    }

    /** Records an additional safely captured value associated with this span. */
    public WalaruSpan capture(String attribute, Object value) {
        if (id != null && !closed.get()) {
            RuntimeBridge.spanValue(id, attribute, value, false, caller);
        }
        return this;
    }

    /** Records an attribute as present while ensuring its value never reaches the backend. */
    public WalaruSpan captureRedacted(String attribute, Object value) {
        if (id != null && !closed.get()) {
            RuntimeBridge.spanValue(id, attribute, null, true, caller);
        }
        return this;
    }

    /** Marks this span as failed. Calling it repeatedly keeps the first failure. */
    public WalaruSpan failed(Throwable cause) {
        if (id != null && failure == null) failure = cause;
        return this;
    }

    /** Ends the span. Closing more than once has no effect. */
    @Override
    public void close() {
        if (id == null || !closed.compareAndSet(false, true)) return;
        long duration = Math.max(0L, System.nanoTime() - startedNanos);
        RuntimeBridge.spanFinished(id, name, duration, failure, caller);
    }
}
