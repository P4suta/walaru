package io.github.p4suta.walaru;

import java.util.Objects;
import java.util.concurrent.Callable;
import java.util.concurrent.Executor;
import java.util.function.Supplier;

/**
 * Opaque test-observation context for work submitted to threads that predate the current test.
 *
 * <p>Virtual threads and newly created platform threads normally inherit the active context. Use
 * this type for common pools, long-lived executors, callbacks, and other pre-existing threads.
 */
public final class WalaruContext {
    private static final WalaruContext EMPTY = new WalaruContext(null);

    private final Object token;

    private WalaruContext(Object token) {
        this.token = token;
    }

    static WalaruContext capture() {
        Object token = RuntimeBridge.captureContext();
        return token == null ? EMPTY : new WalaruContext(token);
    }

    /** Returns whether this snapshot contains an active Walaru test context. */
    public boolean present() {
        return RuntimeBridge.contextActive(token);
    }

    /** Wraps a task so the captured context is installed only for its execution. */
    public Runnable wrap(Runnable action) {
        Objects.requireNonNull(action, "action");
        if (!present()) return action;
        return () -> {
            Object previous = RuntimeBridge.swapContext(token);
            try {
                action.run();
            } finally {
                RuntimeBridge.swapContext(previous);
            }
        };
    }

    /** Wraps a checked task so the captured context is installed only for its execution. */
    public <T> Callable<T> wrapCallable(Callable<T> action) {
        Objects.requireNonNull(action, "action");
        if (!present()) return action;
        return () -> {
            Object previous = RuntimeBridge.swapContext(token);
            try {
                return action.call();
            } finally {
                RuntimeBridge.swapContext(previous);
            }
        };
    }

    /** Wraps a supplier for use with {@code CompletableFuture} and similar APIs. */
    public <T> Supplier<T> wrapSupplier(Supplier<T> action) {
        Objects.requireNonNull(action, "action");
        if (!present()) return action;
        return () -> {
            Object previous = RuntimeBridge.swapContext(token);
            try {
                return action.get();
            } finally {
                RuntimeBridge.swapContext(previous);
            }
        };
    }

    /** Decorates an executor so every submitted task receives this snapshot. */
    public Executor wrap(Executor executor) {
        Objects.requireNonNull(executor, "executor");
        return command -> executor.execute(wrap(command));
    }
}
