package io.github.p4suta.walaru;

import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.util.concurrent.atomic.AtomicLong;

/** Reflection-only boundary that keeps the public API independent from the agent artifact. */
final class RuntimeBridge {
    private static final String AGENT_BRIDGE = "io.github.p4suta.walaru.agent.AgentBridge";
    private static final StackWalker WALKER = StackWalker.getInstance(StackWalker.Option.RETAIN_CLASS_REFERENCE);
    private static final AtomicLong SPANS = new AtomicLong();
    private static final Backend BACKEND = Backend.discover();

    private RuntimeBridge() {}

    static boolean active() {
        return BACKEND.active();
    }

    static Caller caller() {
        if (!active()) return Caller.UNKNOWN;
        try {
            return WALKER.walk(frames -> frames
                    .filter(frame -> !isApiFrame(frame.getClassName()))
                    .findFirst()
                    .map(frame -> new Caller(
                            frame.getClassName(),
                            frame.getMethodName(),
                            frame.getFileName() == null ? "Unknown.java" : frame.getFileName(),
                            Math.max(1, frame.getLineNumber())))
                    .orElse(Caller.UNKNOWN));
        } catch (Throwable ignored) {
            return Caller.UNKNOWN;
        }
    }

    private static boolean isApiFrame(String className) {
        return className.equals(Walaru.class.getName())
                || className.equals(WalaruSpan.class.getName())
                || className.equals(RuntimeBridge.class.getName());
    }

    static String nextSpanId() {
        return "span-" + SPANS.incrementAndGet();
    }

    static Object captureContext() {
        return BACKEND.invokeResult(BACKEND.captureContext);
    }

    static boolean contextActive(Object context) {
        return BACKEND.contextActive(context);
    }

    static Object swapContext(Object context) {
        return BACKEND.invokeResult(BACKEND.swapContext, context);
    }

    static void capture(String name, Object value, boolean redacted, Caller caller) {
        BACKEND.invoke(BACKEND.capture, name, value, redacted, caller);
    }

    static void checkpoint(String name, Object value, boolean hasValue, Caller caller) {
        BACKEND.invoke(BACKEND.checkpoint, name, value, hasValue, caller);
    }

    static void note(String name, String message, Caller caller) {
        BACKEND.invoke(BACKEND.note, name, message, caller);
    }

    static void spanStarted(String id, String name, Caller caller) {
        BACKEND.invoke(BACKEND.spanStarted, id, name, caller);
    }

    static void spanValue(
            String id, String name, Object value, boolean redacted, Caller caller) {
        BACKEND.invoke(BACKEND.spanValue, id, name, value, redacted, caller);
    }

    static void spanFinished(
            String id, String name, long durationNanos, Throwable failure, Caller caller) {
        BACKEND.invoke(BACKEND.spanFinished, id, name, durationNanos, failure, caller);
    }

    static void testStarted(String uniqueId, String publicName) {
        BACKEND.invoke(BACKEND.testStarted, uniqueId, publicName);
    }

    static void testFinished(String uniqueId, String status, Throwable failure) {
        BACKEND.invoke(BACKEND.testFinished, uniqueId, status, failure);
    }

    record Caller(String className, String method, String file, int line) {
        private static final Caller UNKNOWN = new Caller("unknown", "unknown", "Unknown.java", 1);
    }

    private static final class Backend {
        private final MethodHandle active;
        private final MethodHandle capture;
        private final MethodHandle checkpoint;
        private final MethodHandle note;
        private final MethodHandle spanStarted;
        private final MethodHandle spanValue;
        private final MethodHandle spanFinished;
        private final MethodHandle testStarted;
        private final MethodHandle testFinished;
        private final MethodHandle captureContext;
        private final MethodHandle contextActive;
        private final MethodHandle swapContext;

        private Backend(
                MethodHandle active,
                MethodHandle capture,
                MethodHandle checkpoint,
                MethodHandle note,
                MethodHandle spanStarted,
                MethodHandle spanValue,
                MethodHandle spanFinished,
                MethodHandle testStarted,
                MethodHandle testFinished,
                MethodHandle captureContext,
                MethodHandle contextActive,
                MethodHandle swapContext) {
            this.active = active;
            this.capture = capture;
            this.checkpoint = checkpoint;
            this.note = note;
            this.spanStarted = spanStarted;
            this.spanValue = spanValue;
            this.spanFinished = spanFinished;
            this.testStarted = testStarted;
            this.testFinished = testFinished;
            this.captureContext = captureContext;
            this.contextActive = contextActive;
            this.swapContext = swapContext;
        }

        static Backend discover() {
            try {
                Class<?> bridge = Class.forName(AGENT_BRIDGE, false, ClassLoader.getSystemClassLoader());
                MethodHandles.Lookup lookup = MethodHandles.publicLookup();
                MethodType source = MethodType.methodType(
                        void.class,
                        String.class,
                        Object.class,
                        boolean.class,
                        String.class,
                        String.class,
                        String.class,
                        int.class);
                MethodType namedSource = MethodType.methodType(
                        void.class,
                        String.class,
                        String.class,
                        String.class,
                        String.class,
                        String.class,
                        int.class);
                return new Backend(
                        lookup.findStatic(bridge, "apiActive", MethodType.methodType(boolean.class)),
                        lookup.findStatic(bridge, "apiCapture", source),
                        lookup.findStatic(bridge, "apiCheckpoint", source),
                        lookup.findStatic(bridge, "apiNote", namedSource),
                        lookup.findStatic(bridge, "apiSpanStarted", namedSource),
                        lookup.findStatic(
                                bridge,
                                "apiSpanValue",
                                MethodType.methodType(
                                        void.class,
                                        String.class,
                                        String.class,
                                        Object.class,
                                        boolean.class,
                                        String.class,
                                        String.class,
                                        String.class,
                                        int.class)),
                        lookup.findStatic(
                                bridge,
                                "apiSpanFinished",
                                MethodType.methodType(
                                        void.class,
                                        String.class,
                                        String.class,
                                        long.class,
                                        Throwable.class,
                                        String.class,
                                        String.class,
                                        String.class,
                                        int.class)),
                        lookup.findStatic(
                                bridge,
                                "testStarted",
                                MethodType.methodType(void.class, String.class, String.class)),
                        lookup.findStatic(
                                bridge,
                                "testFinished",
                                MethodType.methodType(
                                        void.class, String.class, String.class, Throwable.class)),
                        optionalStatic(
                                lookup,
                                bridge,
                                "apiCaptureContext",
                                MethodType.methodType(Object.class)),
                        optionalStatic(
                                lookup,
                                bridge,
                                "apiContextActive",
                                MethodType.methodType(boolean.class, Object.class)),
                        optionalStatic(
                                lookup,
                                bridge,
                                "apiSwapContext",
                                MethodType.methodType(Object.class, Object.class)));
            } catch (Throwable unavailable) {
                return new Backend(
                        null, null, null, null, null, null, null, null, null, null, null, null);
            }
        }

        private static MethodHandle optionalStatic(
                MethodHandles.Lookup lookup, Class<?> owner, String name, MethodType type) {
            try {
                return lookup.findStatic(owner, name, type);
            } catch (ReflectiveOperationException unavailable) {
                return null;
            }
        }

        boolean active() {
            if (active == null) return false;
            try {
                return (boolean) active.invokeExact();
            } catch (Throwable ignored) {
                return false;
            }
        }

        void invoke(MethodHandle method, Object... arguments) {
            if (method == null) return;
            try {
                method.invokeWithArguments(arguments);
            } catch (Throwable ignored) {
                // Observability is deliberately fail-open.
            }
        }

        Object invokeResult(MethodHandle method, Object... arguments) {
            if (method == null) return null;
            try {
                return method.invokeWithArguments(arguments);
            } catch (Throwable ignored) {
                return null;
            }
        }

        boolean contextActive(Object context) {
            if (context == null) return false;
            if (contextActive == null) return true;
            return Boolean.TRUE.equals(invokeResult(contextActive, context));
        }

        void invoke(MethodHandle method, String first, Object value, boolean flag, Caller caller) {
            invoke(method, first, value, flag, caller.className(), caller.method(), caller.file(), caller.line());
        }

        void invoke(MethodHandle method, String first, String second, Caller caller) {
            invoke(method, first, second, caller.className(), caller.method(), caller.file(), caller.line());
        }

        void invoke(MethodHandle method, String first, String second) {
            invoke(method, (Object) first, second);
        }

        void invoke(MethodHandle method, String first, String second, Throwable failure) {
            invoke(method, (Object) first, second, failure);
        }

        void invoke(
                MethodHandle method,
                String first,
                String second,
                Object value,
                boolean flag,
                Caller caller) {
            invoke(
                    method,
                    first,
                    second,
                    value,
                    flag,
                    caller.className(),
                    caller.method(),
                    caller.file(),
                    caller.line());
        }

        void invoke(
                MethodHandle method,
                String first,
                String second,
                long duration,
                Throwable failure,
                Caller caller) {
            invoke(
                    method,
                    first,
                    second,
                    duration,
                    failure,
                    caller.className(),
                    caller.method(),
                    caller.file(),
                    caller.line());
        }
    }
}
