package io.github.p4suta.walaru.agent;

import java.io.IOException;
import java.lang.reflect.Array;
import java.nio.charset.StandardCharsets;
import java.nio.charset.Charset;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.regex.Pattern;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;
import java.util.Random;
import java.util.UUID;
import java.util.Base64;
import java.time.Instant;
import java.time.LocalDate;
import java.time.LocalDateTime;

/** Runtime boundary called only from instrumented bytecode and framework listeners. */
public final class AgentBridge {
    private static final int MAX_CAPTURED_FILE_BYTES = 1024 * 1024;
    private static final InheritableThreadLocal<TestContext> CURRENT_TEST = new InheritableThreadLocal<>();
    private static final AtomicLong SEQUENCE = new AtomicLong();
    private static final Map<String, TestContext> ACTIVE_TESTS = new ConcurrentHashMap<>();
    private static final Set<String> FAST_COVERAGE = ConcurrentHashMap.newKeySet();
    private static final Set<String> FAST_DEPENDENCIES = ConcurrentHashMap.newKeySet();
    private static final Pattern SECRET_MESSAGE = Pattern.compile(
            "(?i)(password|secret|token|credential)(\\s*[=:]\\s*)[^\\s,;]+");
    private static volatile EventWriter events;
    private static volatile AgentMode mode = AgentMode.FAST;
    private static volatile String projectPath = ":";
    private static volatile InputTape inputTape = InputTape.disabled();
    private static volatile ReplayScheduler replayScheduler = ReplayScheduler.disabled();

    private AgentBridge() {}

    static synchronized void initializeFromProperties() {
        String eventFile = System.getProperty("walaru.eventFile", "");
        if (eventFile.isBlank()) return;
        reset(Path.of(eventFile), AgentMode.parse(System.getProperty("walaru.mode", "fast")));
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            try {
                close();
            } catch (IOException ignored) {
                // Worker process outcome remains authoritative.
            }
        }, "walaru-event-close"));
    }

    static synchronized void resetForTest(Path path, AgentMode requestedMode) throws IOException {
        reset(path, requestedMode);
    }

    static synchronized void closeForTest() throws IOException {
        close();
    }

    private static void reset(Path path, AgentMode requestedMode) {
        try {
            close();
            events = new EventWriter(path);
            mode = requestedMode;
            projectPath = System.getProperty("walaru.projectPath", ":");
            inputTape = InputTape.fromProperties();
            replayScheduler = ReplayScheduler.fromProperties();
            SEQUENCE.set(0);
            FAST_COVERAGE.clear();
            FAST_DEPENDENCIES.clear();
        } catch (IOException failure) {
            throw new IllegalStateException("cannot initialize Walaru event stream", failure);
        }
    }

    private static void close() throws IOException {
        EventWriter writer = events;
        events = null;
        InputTape tape = inputTape;
        inputTape = InputTape.disabled();
        replayScheduler = ReplayScheduler.disabled();
        Throwable firstFailure = null;
        try {
            if (writer != null) writer.close();
        } catch (Throwable failure) {
            firstFailure = failure;
        }
        try {
            if (tape != null) tape.close();
        } catch (Throwable failure) {
            if (firstFailure == null) firstFailure = failure;
            else firstFailure.addSuppressed(failure);
        } finally {
            ACTIVE_TESTS.values().forEach(TestContext::deactivate);
            ACTIVE_TESTS.clear();
            CURRENT_TEST.remove();
        }
        if (firstFailure instanceof IOException failure) throw failure;
        if (firstFailure instanceof RuntimeException failure) throw failure;
        if (firstFailure instanceof Error failure) throw failure;
        if (firstFailure != null) throw new IOException("cannot close Walaru runtime", firstFailure);
    }

    public static void testStarted(String uniqueId, String publicName) {
        String qualifiedName = qualify(publicName);
        TestContext context = new TestContext(uniqueId, qualifiedName, currentTest());
        TestContext replaced = ACTIVE_TESTS.put(uniqueId, context);
        if (replaced != null) replaced.deactivate();
        CURRENT_TEST.set(context);
        emitForTest(context, "TEST_START", Map.of("testId", uniqueId, "testName", qualifiedName));
    }

    public static void testFinished(String uniqueId, String status, Throwable failure) {
        TestContext context = ACTIVE_TESTS.remove(uniqueId);
        TestContext local = CURRENT_TEST.get();
        if (context == null && local != null && Objects.equals(local.uniqueId, uniqueId)) {
            context = local;
        }
        try {
            Map<String, Object> fields = new LinkedHashMap<>();
            fields.put("testId", uniqueId);
            if (context != null) fields.put("testName", context.publicName);
            fields.put("status", status);
            if (failure != null) {
                fields.put("failureType", failure.getClass().getName());
                boolean trusted = trustedThrowable(failure);
                fields.put(
                        "message",
                        trusted
                                ? redactMessage(failure.getMessage())
                                : "<message unavailable without invoking user code>");
                List<String> frames = new ArrayList<>();
                if (trusted) {
                    for (StackTraceElement frame : failure.getStackTrace()) {
                        if (frames.size() >= 64) break;
                        frames.add(frame.toString());
                    }
                }
                fields.put("frames", frames);
            }
            emitForTest(context, "TEST_FINISH", fields);
        } finally {
            if (context != null) {
                context.deactivate();
            }
            if (local == context || (local != null && Objects.equals(local.uniqueId, uniqueId))) {
                if (local.previous != null && local.previous.active()) {
                    CURRENT_TEST.set(local.previous);
                } else {
                    CURRENT_TEST.remove();
                }
            }
        }
    }

    public static void methodEntered(
            String owner, String method, String descriptor, String path, int line, Object[] arguments) {
        TestContext test = currentTest();
        if (test == null) return;
        if (mode == AgentMode.FAST
                && !FAST_DEPENDENCIES.add(
                        "method\0" + test.uniqueId + '\0' + owner + '\0' + method + '\0' + descriptor)) return;
        List<Object> values = new ArrayList<>();
        if (mode == AgentMode.FULL) {
            for (Object argument : arguments) values.add(SafeValue.capture(argument));
        }
        emitCode("METHOD_ENTER", owner, method, descriptor, path, line, Map.of("values", values));
    }

    public static void methodExited(
            String owner, String method, String descriptor, String path, int line, boolean threw) {
        if (currentTest() == null || mode != AgentMode.FULL) return;
        emitCode("METHOD_EXIT", owner, method, descriptor, path, line, Map.of("threw", threw));
    }

    public static void line(
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            Object receiver,
            Object[] arguments) {
        TestContext test = currentTest();
        if (test == null) return;
        String key = test.uniqueId + '\0' + path + '\0' + line;
        if (mode == AgentMode.FAST && !FAST_COVERAGE.add(key)) return;
        Map<String, Object> values = new LinkedHashMap<>();
        if (mode == AgentMode.FULL) {
            values.put("this", SafeValue.capture(receiver));
            List<Object> capturedArguments = new ArrayList<>();
            for (Object argument : arguments) capturedArguments.add(SafeValue.capture(argument));
            values.put("arguments", capturedArguments);
        }
        emitCode("LINE", owner, method, descriptor, path, line, Map.of("values", values));
    }

    public static void call(
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            String targetOwner,
            String targetMethod,
            String targetDescriptor) {
        TestContext test = currentTest();
        if (test == null) return;
        if (mode == AgentMode.FAST
                && !FAST_DEPENDENCIES.add("call\0"
                        + test.uniqueId
                        + '\0'
                        + owner
                        + '\0'
                        + method
                        + '\0'
                        + targetOwner
                        + '\0'
                        + targetMethod
                        + '\0'
                        + targetDescriptor)) return;
        emitCode(
                "CALL",
                owner,
                method,
                descriptor,
                path,
                line,
                Map.of(
                        "targetOwner", targetOwner,
                        "targetMethod", targetMethod,
                        "targetDescriptor", targetDescriptor));
    }

    public static void write(
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            String targetOwner,
            String field,
            String fieldDescriptor,
            Object receiver,
            Object value,
            boolean volatileField,
            boolean staticField) {
        if (currentTest() == null) return;
        Map<String, Object> values = new LinkedHashMap<>();
        if (mode == AgentMode.FULL) {
            values.put("receiver", SafeValue.capture(receiver));
            values.put("value", SafeValue.captureNamed(field, value));
        }
        values.put("targetKind", "field");
        values.put("targetOwner", targetOwner);
        values.put("field", field);
        values.put("fieldDescriptor", fieldDescriptor);
        values.put("volatile", volatileField);
        values.put("static", staticField);
        emitCode(
                "WRITE",
                owner,
                method,
                descriptor,
                path,
                line,
                Map.of(
                        "targetKind", "field",
                        "targetOwner", targetOwner,
                        "field", field,
                        "fieldDescriptor", fieldDescriptor,
                        "volatile", volatileField,
                        "static", staticField,
                        "values", values));
    }

    public static void read(
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            String targetOwner,
            String field,
            String fieldDescriptor,
            Object receiver,
            Object value,
            boolean volatileField,
            boolean staticField) {
        if (currentTest() == null || mode != AgentMode.FULL) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("receiver", SafeValue.capture(receiver));
        values.put("value", SafeValue.captureNamed(field, value));
        values.put("targetKind", "field");
        values.put("targetOwner", targetOwner);
        values.put("field", field);
        values.put("fieldDescriptor", fieldDescriptor);
        values.put("volatile", volatileField);
        values.put("static", staticField);
        emitCode(
                "READ",
                owner,
                method,
                descriptor,
                path,
                line,
                Map.of(
                        "targetKind", "field",
                        "targetOwner", targetOwner,
                        "field", field,
                        "fieldDescriptor", fieldDescriptor,
                        "volatile", volatileField,
                        "static", staticField,
                        "values", values));
    }

    public static void arrayWrite(
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            Object array,
            int index,
            Object value) {
        if (currentTest() == null || mode != AgentMode.FULL) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("receiver", SafeValue.capture(array));
        values.put("value", SafeValue.capture(value));
        values.put("targetKind", "array");
        values.put("targetOwner", array == null ? "array" : array.getClass().getTypeName());
        values.put("index", index);
        emitCode(
                "WRITE",
                owner,
                method,
                descriptor,
                path,
                line,
                Map.of(
                        "targetKind", "array",
                        "targetOwner", array == null ? "array" : array.getClass().getTypeName(),
                        "index", index,
                        "values", values));
    }

    public static void arrayRead(
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            Object array,
            int index,
            Object value) {
        if (currentTest() == null || mode != AgentMode.FULL) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("targetKind", "array");
        values.put("targetOwner", array == null ? "array" : array.getClass().getTypeName());
        values.put("index", index);
        values.put("value", SafeValue.capture(value));
        emitCode(
                "READ",
                owner,
                method,
                descriptor,
                path,
                line,
                Map.of(
                        "targetKind", "array",
                        "targetOwner", array == null ? "array" : array.getClass().getTypeName(),
                        "index", index,
                        "values", values));
    }

    public static void monitor(
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            String action,
            String monitorKind,
            Object receiver) {
        if (currentTest() == null || mode != AgentMode.FULL) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("action", action);
        values.put("monitorKind", monitorKind);
        values.put("monitor", SafeValue.capture(receiver));
        emitCode(
                "MONITOR",
                owner,
                method,
                descriptor,
                path,
                line,
                Map.of(
                        "action", action,
                        "monitorKind", monitorKind,
                        "values", values));
    }

    /** Public reflection boundary used by the zero-dependency API artifact. */
    public static boolean apiActive() {
        return events != null && currentTest() != null;
    }

    /** Returns an opaque context token used only by the zero-dependency public API. */
    public static Object apiCaptureContext() {
        return events == null ? null : currentTest();
    }

    /** Returns whether an opaque public-API context still belongs to a running test. */
    public static boolean apiContextActive(Object context) {
        return events != null && context instanceof TestContext test && test.active();
    }

    /** Installs an opaque context token and returns the previous token for structured restoration. */
    public static Object apiSwapContext(Object replacement) {
        TestContext previous = currentTest();
        if (events != null && replacement instanceof TestContext context && context.active()) {
            CURRENT_TEST.set(context);
        } else {
            CURRENT_TEST.remove();
        }
        return previous;
    }

    /** Records one explicitly named value without linking the API artifact to the agent. */
    public static void apiCapture(
            String name,
            Object value,
            boolean redacted,
            String className,
            String method,
            String path,
            int line) {
        if (!apiActive()) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("name", boundedText(name, 256));
        values.put("value", redacted ? "<redacted>" : SafeValue.captureNamed(name, value));
        values.put("redacted", redacted);
        emitApiCode("CAPTURE", className, method, path, line, Map.of("values", values));
    }

    /** Records an explicit logical checkpoint and an optional safe value. */
    public static void apiCheckpoint(
            String name,
            Object value,
            boolean hasValue,
            String className,
            String method,
            String path,
            int line) {
        if (!apiActive()) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("name", boundedText(name, 256));
        if (hasValue) values.put("value", SafeValue.captureNamed(name, value));
        emitApiCode("CHECKPOINT", className, method, path, line, Map.of("values", values));
    }

    /** Records a short user annotation. */
    public static void apiNote(
            String name, String message, String className, String method, String path, int line) {
        if (!apiActive()) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("name", boundedText(name, 256));
        values.put("message", SafeValue.captureNamed(name, message));
        emitApiCode("NOTE", className, method, path, line, Map.of("values", values));
    }

    /** Starts an explicit user span. */
    public static void apiSpanStarted(
            String spanId, String name, String className, String method, String path, int line) {
        if (!apiActive()) return;
        emitApiCode(
                "SPAN_START",
                className,
                method,
                path,
                line,
                Map.of("values", Map.of("spanId", boundedText(spanId, 128), "name", boundedText(name, 256))));
    }

    /** Adds a safely captured value to an explicit user span. */
    public static void apiSpanValue(
            String spanId,
            String name,
            Object value,
            boolean redacted,
            String className,
            String method,
            String path,
            int line) {
        if (!apiActive()) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("spanId", boundedText(spanId, 128));
        values.put("name", boundedText(name, 256));
        values.put("value", redacted ? "<redacted>" : SafeValue.captureNamed(name, value));
        values.put("redacted", redacted);
        emitApiCode("SPAN_VALUE", className, method, path, line, Map.of("values", values));
    }

    /** Ends an explicit user span. Duration is observational and excluded from replay identity. */
    public static void apiSpanFinished(
            String spanId,
            String name,
            long durationNanos,
            Throwable failure,
            String className,
            String method,
            String path,
            int line) {
        if (!apiActive()) return;
        Map<String, Object> values = new LinkedHashMap<>();
        values.put("spanId", boundedText(spanId, 128));
        values.put("name", boundedText(name, 256));
        values.put("status", failure == null ? "ok" : "failed");
        if (failure != null) {
            values.put("failureType", failure.getClass().getName());
            values.put(
                    "message",
                    trustedThrowable(failure)
                            ? redactMessage(failure.getMessage())
                            : "<message unavailable without invoking user code>");
        }
        emitApiCode(
                "SPAN_END",
                className,
                method,
                path,
                line,
                Map.of(
                        "values", values,
                        "observations", Map.of("durationNanos", Math.max(0L, durationNanos))));
    }

    public static void capabilityMissing(String capability, String reason) {
        if (currentTest() != null) {
            emit("CAPABILITY", Map.of("capability", capability, "available", false, "reason", reason));
        }
    }

    public static long currentTimeMillis() {
        if (currentTest() == null) return System.currentTimeMillis();
        return Long.parseLong(inputTape.next("time.currentTimeMillis", () -> Long.toString(System.currentTimeMillis())));
    }

    public static long nanoTime() {
        if (currentTest() == null) return System.nanoTime();
        return Long.parseLong(inputTape.next("time.nanoTime", () -> Long.toString(System.nanoTime())));
    }

    public static UUID randomUuid() {
        if (currentTest() == null) return UUID.randomUUID();
        return UUID.fromString(inputTape.next("uuid.random", () -> UUID.randomUUID().toString()));
    }

    public static double mathRandom() {
        if (currentTest() == null) return Math.random();
        return Double.parseDouble(inputTape.next("random.math", () -> Double.toHexString(Math.random())));
    }

    public static int randomNextInt(Random random, int bound) {
        if (currentTest() == null) return random.nextInt(bound);
        return Integer.parseInt(inputTape.next("random.nextInt.bound", () -> Integer.toString(random.nextInt(bound))));
    }

    public static int randomNextInt(Random random) {
        if (currentTest() == null) return random.nextInt();
        return Integer.parseInt(inputTape.next("random.nextInt", () -> Integer.toString(random.nextInt())));
    }

    public static long randomNextLong(Random random) {
        if (currentTest() == null) return random.nextLong();
        return Long.parseLong(inputTape.next("random.nextLong", () -> Long.toString(random.nextLong())));
    }

    public static long randomNextLong(Random random, long bound) {
        if (currentTest() == null) return random.nextLong(bound);
        return Long.parseLong(inputTape.next("random.nextLong.bound", () -> Long.toString(random.nextLong(bound))));
    }

    public static boolean randomNextBoolean(Random random) {
        if (currentTest() == null) return random.nextBoolean();
        return Boolean.parseBoolean(
                inputTape.next("random.nextBoolean", () -> Boolean.toString(random.nextBoolean())));
    }

    public static float randomNextFloat(Random random) {
        if (currentTest() == null) return random.nextFloat();
        return Float.valueOf(inputTape.next("random.nextFloat", () -> Float.toHexString(random.nextFloat())));
    }

    public static double randomNextDouble(Random random) {
        if (currentTest() == null) return random.nextDouble();
        return Double.valueOf(inputTape.next("random.nextDouble", () -> Double.toHexString(random.nextDouble())));
    }

    public static double randomNextGaussian(Random random) {
        if (currentTest() == null) return random.nextGaussian();
        return Double.valueOf(inputTape.next("random.nextGaussian", () -> Double.toHexString(random.nextGaussian())));
    }

    public static void randomNextBytes(Random random, byte[] destination) {
        if (currentTest() == null) {
            random.nextBytes(destination);
            return;
        }
        String encoded = inputTape.next("random.nextBytes", () -> {
            random.nextBytes(destination);
            return Base64.getEncoder().encodeToString(destination);
        });
        byte[] replayed = Base64.getDecoder().decode(encoded);
        if (replayed.length != destination.length) {
            throw new IllegalStateException("recorded random byte count changed");
        }
        System.arraycopy(replayed, 0, destination, 0, replayed.length);
    }

    public static Instant instantNow() {
        if (currentTest() == null) return Instant.now();
        return Instant.parse(inputTape.next("time.instant", () -> Instant.now().toString()));
    }

    public static LocalDate localDateNow() {
        if (currentTest() == null) return LocalDate.now();
        return LocalDate.parse(inputTape.next("time.localDate", () -> LocalDate.now().toString()));
    }

    public static LocalDateTime localDateTimeNow() {
        if (currentTest() == null) return LocalDateTime.now();
        return LocalDateTime.parse(inputTape.next("time.localDateTime", () -> LocalDateTime.now().toString()));
    }

    public static byte[] fileReadAllBytes(Path path) throws IOException {
        if (currentTest() == null) return Files.readAllBytes(path);
        return inputTape.nextBytes(
                "io.file.readAllBytes." + fileKey(path),
                () -> Files.readAllBytes(path),
                MAX_CAPTURED_FILE_BYTES);
    }

    public static String fileReadString(Path path) throws IOException {
        if (currentTest() == null) return Files.readString(path);
        return new String(
                inputTape.nextBytes(
                        "io.file.readString." + fileKey(path),
                        () -> Files.readAllBytes(path),
                        MAX_CAPTURED_FILE_BYTES),
                StandardCharsets.UTF_8);
    }

    public static String fileReadString(Path path, Charset charset) throws IOException {
        Objects.requireNonNull(charset, "charset");
        if (currentTest() == null) return Files.readString(path, charset);
        String charsetName = charset.name();
        return new String(
                inputTape.nextBytes(
                        "io.file.readStringCharset." + fileKey(path) + '.' + shortDigest(charsetName),
                        () -> Files.readAllBytes(path),
                        MAX_CAPTURED_FILE_BYTES),
                charset);
    }

    static void inputObserved(String kind, String encoded, String value) {
        if (currentTest() != null) {
            emit("INPUT", Map.of("values", Map.of("kind", kind, "encoded", encoded, "value", value)));
        }
    }

    static void inputObservedSensitive(String kind, int byteCount) {
        if (currentTest() != null) {
            emit("INPUT", Map.of("values", Map.of(
                    "kind", kind,
                    "sensitive", true,
                    "value", "<redacted:file-input " + byteCount + " bytes>")));
        }
    }

    private static String fileKey(Path path) {
        return shortDigest(path.toAbsolutePath().normalize().toString().replace('\\', '/'));
    }

    private static String shortDigest(String value) {
        try {
            byte[] digest = MessageDigest.getInstance("SHA-256")
                    .digest(value.getBytes(StandardCharsets.UTF_8));
            StringBuilder result = new StringBuilder(24);
            for (int index = 0; index < 12; index++) result.append(String.format("%02x", digest[index]));
            return result.toString();
        } catch (NoSuchAlgorithmException impossible) {
            throw new IllegalStateException(impossible);
        }
    }

    private static void emitCode(
            String type,
            String owner,
            String method,
            String descriptor,
            String path,
            int line,
            Map<String, ?> extra) {
        Map<String, Object> fields = new LinkedHashMap<>();
        fields.put("owner", owner);
        fields.put("method", logicalMethod(owner, method));
        fields.put("descriptor", descriptor);
        fields.put("path", path);
        fields.put("line", Math.max(1, line));
        boolean coroutine = method.equals("invokeSuspend")
                || descriptor.contains("kotlin/coroutines/Continuation");
        if (coroutine) {
            fields.put("coroutine", true);
            fields.put("logicalStack", logicalStack(owner, method, path, line));
        }
        fields.putAll(extra);
        emit(type, fields);
    }

    private static void emitApiCode(
            String type,
            String className,
            String method,
            String path,
            int line,
            Map<String, ?> extra) {
        String owner = boundedText(className, 512).replace('.', '/');
        emitCode(
                type,
                owner,
                boundedText(method, 256),
                "",
                boundedText(path, 1024),
                line,
                extra);
    }

    private static List<Map<String, Object>> logicalStack(
            String owner, String method, String path, int line) {
        List<Map<String, Object>> frames = new ArrayList<>();
        frames.add(logicalFrame(owner.replace('/', '.'), logicalMethod(owner, method), path, line));
        for (StackTraceElement frame : Thread.currentThread().getStackTrace()) {
            if (frames.size() >= 64 || infrastructureFrame(frame.getClassName())) continue;
            frames.add(logicalFrame(
                    frame.getClassName(),
                    logicalMethod(frame.getClassName(), frame.getMethodName()),
                    frame.getFileName() == null ? "Unknown.kt" : frame.getFileName(),
                    Math.max(1, frame.getLineNumber())));
        }
        return List.copyOf(frames);
    }

    private static Map<String, Object> logicalFrame(
            String className, String method, String path, int line) {
        Map<String, Object> frame = new LinkedHashMap<>();
        frame.put("className", className);
        frame.put("method", method);
        frame.put("path", path);
        frame.put("line", Math.max(1, line));
        return frame;
    }

    private static boolean infrastructureFrame(String className) {
        return className.startsWith("java.")
                || className.startsWith("jdk.")
                || className.startsWith("sun.")
                || className.startsWith("org.junit.")
                || className.startsWith("org.gradle.")
                || className.startsWith("io.github.p4suta.walaru.agent.");
    }

    private static void emit(String type, Map<String, ?> supplied) {
        TestContext test = currentTest();
        if (CURRENT_TEST.get() != null && test == null) return;
        emitForTest(test, type, supplied);
    }

    private static void emitForTest(TestContext test, String type, Map<String, ?> supplied) {
        EventWriter writer = events;
        if (writer == null) return;
        String threadKey = ReplayScheduler.threadKey();
        replayScheduler.before(type, threadKey);
        try {
            long sequence = SEQUENCE.getAndIncrement();
            Map<String, Object> fields = new LinkedHashMap<>();
            fields.put("schemaVersion", 1);
            fields.put("sequence", sequence);
            fields.put("threadId", Thread.currentThread().threadId());
            fields.put("threadKey", threadKey);
            fields.put("virtualThread", Thread.currentThread().isVirtual());
            fields.put("type", type);
            fields.put("module", projectPath);
            if (test != null) {
                fields.put("testId", test.uniqueId);
                fields.put("testName", test.publicName);
            }
            fields.putAll(supplied);
            fields.put("stateHash", stateHash(fields));
            writer.write(fields);
        } finally {
            replayScheduler.after();
        }
    }

    private static String logicalMethod(String owner, String method) {
        String normalized = method.endsWith("$default")
                ? method.substring(0, method.length() - "$default".length())
                : method;
        if (normalized.equals("invokeSuspend") && owner.contains("$")) {
            String simpleOwner = owner.substring(owner.lastIndexOf('/') + 1);
            String[] segments = simpleOwner.split("\\$");
            for (int index = segments.length - 1; index > 0; index--) {
                if (!segments[index].chars().allMatch(Character::isDigit)) {
                    normalized = segments[index];
                    break;
                }
            }
        }
        return normalized;
    }

    private static String qualify(String publicName) {
        if (projectPath == null || projectPath.isBlank() || projectPath.equals(":")) return publicName;
        return projectPath + "::" + publicName;
    }

    private static boolean trustedThrowable(Throwable failure) {
        String name = failure.getClass().getName();
        return name.startsWith("java.lang.")
                || name.startsWith("org.opentest4j.")
                || name.startsWith("org.junit.");
    }

    private static String redactMessage(String message) {
        if (message == null) return "";
        String bounded = message.length() <= 512 ? message : message.substring(0, 512) + "…";
        return SECRET_MESSAGE.matcher(bounded).replaceAll("$1$2<redacted>");
    }

    private static String stateHash(Map<String, ?> fields) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            Map<String, Object> stable = new LinkedHashMap<>();
            for (Map.Entry<String, ?> field : fields.entrySet()) {
                if (field.getKey().equals("schemaVersion")
                        || field.getKey().equals("sequence")
                        || field.getKey().equals("threadId")
                        || field.getKey().equals("threadKey")
                        || field.getKey().equals("virtualThread")
                        || field.getKey().equals("testId")
                        || field.getKey().equals("observations")) {
                    continue;
                }
                stable.put(field.getKey(), field.getValue());
            }
            updateDigest(digest, stable);
            byte[] result = digest.digest();
            StringBuilder output = new StringBuilder(32);
            for (int index = 0; index < 16; index++) output.append(String.format("%02x", result[index]));
            return output.toString();
        } catch (NoSuchAlgorithmException impossible) {
            throw new IllegalStateException(impossible);
        }
    }

    private static void updateDigest(MessageDigest digest, Object value) {
        if (value == null) {
            digest.update((byte) '0');
        } else if (value instanceof CharSequence text) {
            updateScalar(digest, 's', text.toString());
        } else if (value instanceof Number number) {
            updateScalar(digest, 'n', number.getClass().getName() + ':' + number);
        } else if (value instanceof Boolean bool) {
            updateScalar(digest, 'b', bool.toString());
        } else if (value instanceof Map<?, ?> map) {
            digest.update((byte) '{');
            List<? extends Map.Entry<?, ?>> entries = new ArrayList<>(map.entrySet());
            entries.sort(Comparator.comparing(entry -> stableMapKey(entry.getKey())));
            for (Map.Entry<?, ?> entry : entries) {
                updateDigest(digest, entry.getKey());
                updateDigest(digest, entry.getValue());
            }
            digest.update((byte) '}');
        } else if (value instanceof Iterable<?> iterable) {
            digest.update((byte) '[');
            for (Object item : iterable) updateDigest(digest, item);
            digest.update((byte) ']');
        } else if (value.getClass().isArray()) {
            digest.update((byte) '[');
            for (int index = 0; index < Array.getLength(value); index++) {
                updateDigest(digest, Array.get(value, index));
            }
            digest.update((byte) ']');
        } else {
            updateScalar(digest, 'c', value.getClass().getName());
        }
    }

    private static String stableMapKey(Object key) {
        if (key == null) return "";
        if (key instanceof String text) return "s:" + text;
        return "c:" + key.getClass().getName();
    }

    private static String boundedText(String value, int maxLength) {
        if (value == null || value.isBlank()) return "unknown";
        return value.length() <= maxLength ? value : value.substring(0, maxLength);
    }

    private static void updateScalar(MessageDigest digest, char kind, String value) {
        byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
        digest.update((byte) kind);
        digest.update(Integer.toString(bytes.length).getBytes(StandardCharsets.UTF_8));
        digest.update((byte) ':');
        digest.update(bytes);
    }

    private static TestContext currentTest() {
        TestContext context = CURRENT_TEST.get();
        return context != null && context.active() ? context : null;
    }

    private static final class TestContext {
        private final String uniqueId;
        private final String publicName;
        private final TestContext previous;
        private final AtomicBoolean active = new AtomicBoolean(true);

        private TestContext(String uniqueId, String publicName, TestContext previous) {
            this.uniqueId = uniqueId;
            this.publicName = publicName;
            this.previous = previous;
        }

        private boolean active() {
            return active.get();
        }

        private void deactivate() {
            active.set(false);
        }
    }
}
