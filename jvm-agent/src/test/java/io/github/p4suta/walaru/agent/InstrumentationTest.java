package io.github.p4suta.walaru.agent;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Random;
import java.time.Instant;
import java.time.LocalDate;
import java.time.LocalDateTime;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.concurrent.CountDownLatch;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

final class InstrumentationTest {
    @TempDir
    Path directory;

    @Test
    void stateHashChangesWhenCapturedRuntimeValuesChange() throws Exception {
        String first = methodEnterStateHash(-3, directory.resolve("first.jsonl"));
        String second = methodEnterStateHash(4, directory.resolve("second.jsonl"));

        assertNotEquals(first, second);
    }

    @Test
    void stateHashIgnoresRunLocalSequenceAndThreadIdentity() throws Exception {
        Path events = directory.resolve("identity.jsonl");
        AgentBridge.resetForTest(events, AgentMode.FULL);
        AgentBridge.testStarted("fixture.SampleTest#works", "fixture.SampleTest#works");
        Runnable emit = () -> AgentBridge.methodEntered(
                "fixture/Sample", "run", "(I)V", "Sample.kt", 10, new Object[] {7});
        Thread first = new Thread(emit, "first-replay-thread");
        Thread second = new Thread(emit, "second-replay-thread");
        first.start();
        first.join();
        second.start();
        second.join();
        AgentBridge.testFinished("fixture.SampleTest#works", "passed", null);
        AgentBridge.closeForTest();

        List<String> hashes = Files.readAllLines(events).stream()
                .filter(line -> line.contains("\"type\":\"METHOD_ENTER\""))
                .map(InstrumentationTest::stateHash)
                .toList();
        assertEquals(2, hashes.size());
        assertEquals(hashes.get(0), hashes.get(1));
    }

    @Test
    void stateHashIsIndependentOfMapIterationOrder() throws Exception {
        Method stateHash = AgentBridge.class.getDeclaredMethod("stateHash", Map.class);
        stateHash.setAccessible(true);
        Map<String, Object> left = new LinkedHashMap<>();
        left.put("targetDescriptor", "()V");
        left.put("targetMethod", "run");
        left.put("targetOwner", "fixture/Sample");
        Map<String, Object> right = new LinkedHashMap<>();
        right.put("targetOwner", "fixture/Sample");
        right.put("targetMethod", "run");
        right.put("targetDescriptor", "()V");

        assertEquals(stateHash.invoke(null, left), stateHash.invoke(null, right));
    }

    @Test
    void fullModeEmitsLineCallWriteAndSafeValueEvents() throws Exception {
        Path events = directory.resolve("events.jsonl");
        AgentBridge.resetForTest(events, AgentMode.FULL);
        byte[] instrumented = BytecodeInstrumenter.instrument(sampleClass(), "fixture/Sample", AgentMode.FULL);
        Class<?> sample = new BytesClassLoader().define("fixture.Sample", instrumented);

        AgentBridge.testStarted("fixture.SampleTest#works", "fixture.SampleTest#works");
        Method run = sample.getMethod("run", int.class);
        assertEquals(3, run.invoke(null, -3));
        AgentBridge.testFinished("fixture.SampleTest#works", "passed", null);
        AgentBridge.closeForTest();

        String trace = Files.readString(events);
        assertTrue(trace.contains("\"type\":\"METHOD_ENTER\""), trace);
        assertTrue(trace.contains("\"type\":\"LINE\""), trace);
        assertTrue(trace.contains("\"type\":\"CALL\""), trace);
        assertTrue(trace.contains("\"type\":\"WRITE\""), trace);
        assertTrue(trace.contains("\"path\":\"Sample.kt\""), trace);
        assertTrue(trace.contains("\"values\":[-3]"), trace);
        assertTrue(
                trace.lines()
                        .filter(line -> line.contains("\"type\":\"LINE\""))
                        .anyMatch(line -> line.contains("\"arguments\":[-3]")),
                trace);
        assertTrue(
                trace.lines()
                        .filter(line -> line.contains("\"type\":\"WRITE\""))
                        .anyMatch(line -> line.contains("\"value\":-3")),
                trace);
    }

    @Test
    void fullModeCrossIndexesArrayWritesMonitorsAndVolatileAccesses() throws Exception {
        Path events = directory.resolve("memory-events.jsonl");
        AgentBridge.resetForTest(events, AgentMode.FULL);
        byte[] instrumented = BytecodeInstrumenter.instrument(memoryClass(), "fixture/Memory", AgentMode.FULL);
        Class<?> memory = new BytesClassLoader().define("fixture.Memory", instrumented);

        AgentBridge.testStarted("fixture.MemoryTest#works", "fixture.MemoryTest#works");
        int[] values = new int[2];
        assertEquals(7, memory.getMethod("run", int[].class, Object.class).invoke(null, values, new Object()));
        AgentBridge.testFinished("fixture.MemoryTest#works", "passed", null);
        AgentBridge.closeForTest();

        String trace = Files.readString(events);
        assertTrue(trace.contains("\"type\":\"MONITOR\""), trace);
        assertTrue(trace.contains("\"action\":\"enter\""), trace);
        assertTrue(trace.contains("\"action\":\"exit\""), trace);
        assertTrue(trace.contains("\"monitorKind\":\"synchronizedStaticMethod\""), trace);
        assertTrue(trace.lines().anyMatch(line -> line.contains("\"type\":\"WRITE\"")
                && line.contains("\"targetKind\":\"array\"")
                && line.contains("\"index\":1")
                && line.contains("\"value\":7")), trace);
        assertTrue(trace.lines().anyMatch(line -> line.contains("\"type\":\"WRITE\"")
                && line.contains("\"field\":\"published\"")
                && line.contains("\"volatile\":true")), trace);
        assertTrue(trace.lines().anyMatch(line -> line.contains("\"type\":\"READ\"")
                && line.contains("\"field\":\"published\"")
                && line.contains("\"volatile\":true")), trace);
    }

    @Test
    void kotlinCompilerMethodNamesArePresentedAsLogicalSourceMethods() throws Exception {
        Method logicalMethod = AgentBridge.class.getDeclaredMethod("logicalMethod", String.class, String.class);
        logicalMethod.setAccessible(true);

        assertEquals("fetch", logicalMethod.invoke(null, "demo/FlowKt", "fetch$default"));
        assertEquals("fetch", logicalMethod.invoke(null, "demo/FlowKt$fetch$1", "invokeSuspend"));
        assertEquals("render", logicalMethod.invoke(null, "demo/View$render$2", "invokeSuspend"));
    }

    @Test
    void timeUuidAndRandomInputsReplayExactlyFromTheRecordedTape() throws Exception {
        Path tape = directory.resolve("inputs.tape");
        Path recordedEvents = directory.resolve("recorded-inputs.jsonl");
        System.setProperty("walaru.inputFile", tape.toString());
        try {
            AgentBridge.resetForTest(recordedEvents, AgentMode.FULL);
            Class<?> recordedClass = new BytesClassLoader().define(
                    "fixture.Nondeterministic", BytecodeInstrumenter.instrument(
                            nondeterministicClass(), "fixture/Nondeterministic", AgentMode.FULL));
            AgentBridge.testStarted("fixture.InputTest#works", "fixture.InputTest#works");
            Object[] recorded = (Object[]) recordedClass.getMethod("run", Random.class).invoke(null, new Random(7));
            AgentBridge.testFinished("fixture.InputTest#works", "passed", null);
            AgentBridge.closeForTest();

            System.clearProperty("walaru.inputFile");
            System.setProperty("walaru.replayInputFile", tape.toString());
            Path replayedEvents = directory.resolve("replayed-inputs.jsonl");
            AgentBridge.resetForTest(replayedEvents, AgentMode.FULL);
            Class<?> replayedClass = new BytesClassLoader().define(
                    "fixture.Nondeterministic", BytecodeInstrumenter.instrument(
                            nondeterministicClass(), "fixture/Nondeterministic", AgentMode.FULL));
            AgentBridge.testStarted("fixture.InputTest#works", "fixture.InputTest#works");
            Object[] replayed = (Object[]) replayedClass.getMethod("run", Random.class).invoke(null, new Random(999));
            AgentBridge.testFinished("fixture.InputTest#works", "passed", null);
            AgentBridge.closeForTest();

            assertEquals(List.of(recorded), List.of(replayed));
            assertEquals(3, Files.readAllLines(recordedEvents).stream()
                    .filter(line -> line.contains("\"type\":\"INPUT\""))
                    .count());
            assertEquals(
                    Files.readAllLines(recordedEvents).stream()
                            .filter(line -> line.contains("\"type\":\"INPUT\""))
                            .map(InstrumentationTest::stableInput)
                            .toList(),
                    Files.readAllLines(replayedEvents).stream()
                            .filter(line -> line.contains("\"type\":\"INPUT\""))
                            .map(InstrumentationTest::stableInput)
                            .toList());
        } finally {
            System.clearProperty("walaru.inputFile");
            System.clearProperty("walaru.replayInputFile");
            AgentBridge.closeForTest();
        }
    }

    @Test
    void optedInFileInputIsRedactedAndReplayedAfterTheFileChanges() throws Exception {
        Path source = directory.resolve("external-input.txt");
        Path tape = directory.resolve("file-inputs.tape");
        Files.writeString(source, "token=recorded-secret");
        System.setProperty("walaru.captureFileIo", "true");
        System.setProperty("walaru.inputFile", tape.toString());
        try {
            Path recordedEvents = directory.resolve("file-recorded.jsonl");
            AgentBridge.resetForTest(recordedEvents, AgentMode.FULL);
            Class<?> recordedClass = new BytesClassLoader().define(
                    "fixture.FileInput",
                    BytecodeInstrumenter.instrument(fileInputClass(), "fixture/FileInput", AgentMode.FULL));
            AgentBridge.testStarted("fixture.FileInputTest#works", "fixture.FileInputTest#works");
            assertEquals("token=recorded-secret", recordedClass.getMethod("run", Path.class).invoke(null, source));
            AgentBridge.testFinished("fixture.FileInputTest#works", "passed", null);
            AgentBridge.closeForTest();

            String recordedTrace = Files.readString(recordedEvents);
            assertTrue(recordedTrace.contains("\"kind\":\"io.file.readString."), recordedTrace);
            assertTrue(recordedTrace.contains("<redacted:file-input"), recordedTrace);
            assertFalse(recordedTrace.contains("recorded-secret"), recordedTrace);

            Files.writeString(source, "changed");
            System.clearProperty("walaru.inputFile");
            System.setProperty("walaru.replayInputFile", tape.toString());
            AgentBridge.resetForTest(directory.resolve("file-replayed.jsonl"), AgentMode.FULL);
            Class<?> replayedClass = new BytesClassLoader().define(
                    "fixture.FileInput",
                    BytecodeInstrumenter.instrument(fileInputClass(), "fixture/FileInput", AgentMode.FULL));
            AgentBridge.testStarted("fixture.FileInputTest#works", "fixture.FileInputTest#works");
            assertEquals("token=recorded-secret", replayedClass.getMethod("run", Path.class).invoke(null, source));
            AgentBridge.testFinished("fixture.FileInputTest#works", "passed", null);
            AgentBridge.closeForTest();
        } finally {
            System.clearProperty("walaru.captureFileIo");
            System.clearProperty("walaru.inputFile");
            System.clearProperty("walaru.replayInputFile");
            AgentBridge.closeForTest();
        }
    }

    @Test
    void nullFileCharsetFailsBeforeReadingOrPersistingSensitiveInput() throws Exception {
        Path source = directory.resolve("charset-input.txt");
        Path tape = directory.resolve("charset-inputs.tape");
        Files.writeString(source, "must-not-be-read");
        System.setProperty("walaru.captureFileIo", "true");
        System.setProperty("walaru.inputFile", tape.toString());
        try {
            AgentBridge.resetForTest(directory.resolve("charset-events.jsonl"), AgentMode.FULL);

            assertThrows(NullPointerException.class, () -> AgentBridge.fileReadString(source, null));

            AgentBridge.closeForTest();
            assertTrue(!Files.exists(tape) || Files.size(tape) == 0, "null charset must not consume input");
        } finally {
            System.clearProperty("walaru.captureFileIo");
            System.clearProperty("walaru.inputFile");
            AgentBridge.closeForTest();
        }
    }

    @Test
    void allStandardRandomShapesAndJavaTimeReplayFromOneOrderedTape() throws Exception {
        Path tape = directory.resolve("all-inputs.tape");
        System.setProperty("walaru.inputFile", tape.toString());
        List<Object> recorded;
        try {
            AgentBridge.resetForTest(directory.resolve("all-recorded.jsonl"), AgentMode.FULL);
            AgentBridge.testStarted("fixture.AllInputs#works", "fixture.AllInputs#works");
            recorded = supportedInputs(new Random(11));
            AgentBridge.testFinished("fixture.AllInputs#works", "passed", null);
            AgentBridge.closeForTest();

            System.clearProperty("walaru.inputFile");
            System.setProperty("walaru.replayInputFile", tape.toString());
            AgentBridge.resetForTest(directory.resolve("all-replayed.jsonl"), AgentMode.FULL);
            AgentBridge.testStarted("fixture.AllInputs#works", "fixture.AllInputs#works");
            List<Object> replayed = supportedInputs(new Random(999));
            AgentBridge.testFinished("fixture.AllInputs#works", "passed", null);
            AgentBridge.closeForTest();

            assertEquals(recorded, replayed);
        } finally {
            System.clearProperty("walaru.inputFile");
            System.clearProperty("walaru.replayInputFile");
            AgentBridge.closeForTest();
        }
    }

    @Test
    void recordedThreadScheduleControlsAReplayStartedInTheOppositeOrder() throws Exception {
        Path recordedEvents = directory.resolve("thread-recorded.jsonl");
        AgentBridge.resetForTest(recordedEvents, AgentMode.FULL);
        AgentBridge.testStarted("fixture.Threads#works", "fixture.Threads#works");
        runScheduledThreads(false);
        AgentBridge.testFinished("fixture.Threads#works", "passed", null);
        AgentBridge.closeForTest();

        Path schedule = directory.resolve("thread.schedule");
        Files.write(
                schedule,
                Files.readAllLines(recordedEvents).stream()
                        .map(InstrumentationTest::scheduleEntry)
                        .toList());
        System.setProperty("walaru.replayScheduleFile", schedule.toString());
        try {
            Path replayedEvents = directory.resolve("thread-replayed.jsonl");
            AgentBridge.resetForTest(replayedEvents, AgentMode.FULL);
            AgentBridge.testStarted("fixture.Threads#works", "fixture.Threads#works");
            runScheduledThreads(true);
            AgentBridge.testFinished("fixture.Threads#works", "passed", null);
            AgentBridge.closeForTest();

            assertEquals(lineThreadKeys(recordedEvents), lineThreadKeys(replayedEvents));
        } finally {
            System.clearProperty("walaru.replayScheduleFile");
            AgentBridge.closeForTest();
        }
    }

    @Test
    void namedVirtualThreadsExposeAStableReplayKey() throws Exception {
        Path events = directory.resolve("virtual-thread.jsonl");
        AgentBridge.resetForTest(events, AgentMode.FULL);
        AgentBridge.testStarted("fixture.Virtual#works", "fixture.Virtual#works");
        Thread thread = Thread.ofVirtual().name("virtual-worker").start(() ->
                AgentBridge.line("fixture/Virtual", "run", "()V", "Virtual.java", 7, null, new Object[0]));
        thread.join();
        AgentBridge.testFinished("fixture.Virtual#works", "passed", null);
        AgentBridge.closeForTest();

        String event = Files.readAllLines(events).stream()
                .filter(line -> line.contains("\"type\":\"LINE\""))
                .findFirst()
                .orElseThrow();
        assertTrue(event.contains("\"virtualThread\":true"), event);
        assertTrue(event.contains("\"threadKey\":\"virtual:virtual-worker\""), event);
    }

    @Test
    void suspendStateMachineEventsExposeABoundedKotlinLogicalStack() throws Exception {
        Path events = directory.resolve("coroutine-stack.jsonl");
        AgentBridge.resetForTest(events, AgentMode.FULL);
        AgentBridge.testStarted("fixture.Coroutine#works", "fixture.Coroutine#works");
        AgentBridge.methodEntered(
                "demo/FlowKt$fetch$1",
                "invokeSuspend",
                "(Ljava/lang/Object;)Ljava/lang/Object;",
                "demo/Flow.kt",
                14,
                new Object[] {"continuation"});
        AgentBridge.testFinished("fixture.Coroutine#works", "passed", null);
        AgentBridge.closeForTest();

        String event = Files.readAllLines(events).stream()
                .filter(line -> line.contains("\"type\":\"METHOD_ENTER\""))
                .findFirst()
                .orElseThrow();
        assertTrue(event.contains("\"coroutine\":true"), event);
        assertTrue(event.contains("\"logicalStack\":["), event);
        assertTrue(event.contains("\"method\":\"fetch\""), event);
        assertTrue(event.length() < 32_768, event);
    }

    @Test
    void fastInstrumentationTrimmedMeanOverheadStaysBelowThirtyPercent() throws Exception {
        byte[] originalBytes = fastWorkloadClass();
        Class<?> original = new BytesClassLoader().define("fixture.FastWorkload", originalBytes);
        Class<?> instrumented = new BytesClassLoader().define(
                "fixture.FastWorkload",
                BytecodeInstrumenter.instrument(originalBytes, "fixture/FastWorkload", AgentMode.FAST));
        Method originalRun = original.getMethod("run", int.class);
        Method instrumentedRun = instrumented.getMethod("run", int.class);
        Path events = directory.resolve("fast-overhead.jsonl");
        try {
            AgentBridge.resetForTest(events, AgentMode.FAST);
            AgentBridge.testStarted("fixture.FastWorkloadTest#works", "fixture.FastWorkloadTest#works");
            for (int index = 0; index < 2_000; index++) {
                originalRun.invoke(null, 10_000);
                instrumentedRun.invoke(null, 10_000);
            }
            long[] baseline = new long[9];
            long[] observed = new long[9];
            Object expected = null;
            Object actual = null;
            for (int index = 0; index < baseline.length; index++) {
                if (index % 2 == 0) {
                    long started = System.nanoTime();
                    expected = originalRun.invoke(null, 30_000_000);
                    baseline[index] = System.nanoTime() - started;
                    started = System.nanoTime();
                    actual = instrumentedRun.invoke(null, 30_000_000);
                    observed[index] = System.nanoTime() - started;
                } else {
                    long started = System.nanoTime();
                    actual = instrumentedRun.invoke(null, 30_000_000);
                    observed[index] = System.nanoTime() - started;
                    started = System.nanoTime();
                    expected = originalRun.invoke(null, 30_000_000);
                    baseline[index] = System.nanoTime() - started;
                }
            }
            AgentBridge.testFinished("fixture.FastWorkloadTest#works", "passed", null);
            assertEquals(expected, actual);
            Arrays.sort(baseline);
            Arrays.sort(observed);
            long baselineTrimmedMean = Arrays.stream(baseline, 1, baseline.length - 1).sum()
                    / (baseline.length - 2);
            long observedTrimmedMean = Arrays.stream(observed, 1, observed.length - 1).sum()
                    / (observed.length - 2);
            assertTrue(
                    observedTrimmedMean * 10 <= baselineTrimmedMean * 13,
                    "fast instrumentation overhead exceeded 30% after trimming outliers: baseline="
                            + baselineTrimmedMean
                            + "ns observed="
                            + observedTrimmedMean
                            + "ns");
        } finally {
            AgentBridge.closeForTest();
        }
    }

    private static void runScheduledThreads(boolean startSecondFirst) throws Exception {
        CountDownLatch startA = new CountDownLatch(startSecondFirst ? 1 : 0);
        CountDownLatch startB = new CountDownLatch(startSecondFirst ? 0 : 1);
        CountDownLatch firstA = new CountDownLatch(1);
        CountDownLatch firstB = new CountDownLatch(1);
        Thread a = new Thread(() -> {
            await(startA);
            AgentBridge.line("fixture/Threads", "a", "()V", "Threads.java", 10, null, new Object[0]);
            firstA.countDown();
            await(firstB);
            AgentBridge.line("fixture/Threads", "a", "()V", "Threads.java", 11, null, new Object[0]);
        }, "schedule-a");
        Thread b = new Thread(() -> {
            await(startB);
            if (!startSecondFirst) await(firstA);
            AgentBridge.line("fixture/Threads", "b", "()V", "Threads.java", 20, null, new Object[0]);
            firstB.countDown();
            AgentBridge.line("fixture/Threads", "b", "()V", "Threads.java", 21, null, new Object[0]);
        }, "schedule-b");
        a.start();
        b.start();
        if (startSecondFirst) {
            Thread.sleep(25);
            startA.countDown();
        } else {
            startB.countDown();
        }
        a.join();
        b.join();
    }

    private static void await(CountDownLatch latch) {
        try {
            latch.await();
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new IllegalStateException(interrupted);
        }
    }

    private static String scheduleEntry(String event) {
        String type = jsonString(event, "type");
        String category = switch (type) {
            case "METHOD_ENTER", "METHOD_EXIT", "CALL" -> "CALL";
            default -> type;
        };
        String threadKey = jsonString(event, "threadKey");
        return category + "\t" + HexFormat.of().formatHex(threadKey.getBytes(java.nio.charset.StandardCharsets.UTF_8));
    }

    private static List<String> lineThreadKeys(Path events) throws Exception {
        return Files.readAllLines(events).stream()
                .filter(line -> line.contains("\"type\":\"LINE\""))
                .map(line -> jsonString(line, "threadKey"))
                .toList();
    }

    private static String jsonString(String event, String name) {
        String marker = "\"" + name + "\":\"";
        int markerIndex = event.indexOf(marker);
        if (markerIndex < 0) throw new IllegalArgumentException("missing " + name + " in " + event);
        int start = markerIndex + marker.length();
        return event.substring(start, event.indexOf('"', start));
    }

    private static List<Object> supportedInputs(Random random) {
        byte[] bytes = new byte[12];
        AgentBridge.randomNextBytes(random, bytes);
        return List.of(
                AgentBridge.randomNextInt(random),
                AgentBridge.randomNextInt(random, 50),
                AgentBridge.randomNextLong(random),
                AgentBridge.randomNextLong(random, 500L),
                AgentBridge.randomNextBoolean(random),
                AgentBridge.randomNextFloat(random),
                AgentBridge.randomNextDouble(random),
                AgentBridge.randomNextGaussian(random),
                Arrays.toString(bytes),
                AgentBridge.instantNow(),
                AgentBridge.localDateNow(),
                AgentBridge.localDateTimeNow());
    }

    private static String methodEnterStateHash(int argument, Path events) throws Exception {
        AgentBridge.resetForTest(events, AgentMode.FULL);
        byte[] instrumented = BytecodeInstrumenter.instrument(sampleClass(), "fixture/Sample", AgentMode.FULL);
        Class<?> sample = new BytesClassLoader().define("fixture.Sample", instrumented);
        AgentBridge.testStarted("fixture.SampleTest#works", "fixture.SampleTest#works");
        sample.getMethod("run", int.class).invoke(null, argument);
        AgentBridge.testFinished("fixture.SampleTest#works", "passed", null);
        AgentBridge.closeForTest();

        String event = Files.readAllLines(events).stream()
                .filter(line -> line.contains("\"type\":\"METHOD_ENTER\"")
                        && line.contains("\"owner\":\"fixture/Sample\""))
                .findFirst()
                .orElseThrow();
        return stateHash(event);
    }

    private static String stateHash(String event) {
        String marker = "\"stateHash\":\"";
        int start = event.indexOf(marker) + marker.length();
        return event.substring(start, event.indexOf('"', start));
    }

    private static String stableInput(String event) {
        int type = event.indexOf("\"type\":\"INPUT\"");
        int hash = event.indexOf(",\"stateHash\"");
        return event.substring(type, hash);
    }

    private static byte[] sampleClass() {
        ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        writer.visit(Opcodes.V21, Opcodes.ACC_PUBLIC, "fixture/Sample", null, "java/lang/Object", null);
        writer.visitSource("Sample.kt", null);
        writer.visitField(Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC, "counter", "I", null, null).visitEnd();
        MethodVisitor method = writer.visitMethod(
                Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC, "run", "(I)I", null, null);
        method.visitCode();
        method.visitLineNumber(10, new org.objectweb.asm.Label());
        method.visitVarInsn(Opcodes.ILOAD, 0);
        method.visitFieldInsn(Opcodes.PUTSTATIC, "fixture/Sample", "counter", "I");
        method.visitLineNumber(11, new org.objectweb.asm.Label());
        method.visitVarInsn(Opcodes.ILOAD, 0);
        method.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/Math", "abs", "(I)I", false);
        method.visitInsn(Opcodes.IRETURN);
        method.visitMaxs(0, 0);
        method.visitEnd();
        writer.visitEnd();
        return writer.toByteArray();
    }

    private static byte[] fastWorkloadClass() {
        ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        writer.visit(Opcodes.V21, Opcodes.ACC_PUBLIC, "fixture/FastWorkload", null, "java/lang/Object", null);
        writer.visitSource("FastWorkload.java", null);
        MethodVisitor method = writer.visitMethod(
                Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC, "run", "(I)J", null, null);
        method.visitCode();
        org.objectweb.asm.Label start = new org.objectweb.asm.Label();
        org.objectweb.asm.Label check = new org.objectweb.asm.Label();
        org.objectweb.asm.Label end = new org.objectweb.asm.Label();
        method.visitLabel(start);
        method.visitLineNumber(4, start);
        method.visitInsn(Opcodes.LCONST_1);
        method.visitVarInsn(Opcodes.LSTORE, 1);
        method.visitInsn(Opcodes.ICONST_0);
        method.visitVarInsn(Opcodes.ISTORE, 3);
        method.visitLabel(check);
        method.visitVarInsn(Opcodes.ILOAD, 3);
        method.visitVarInsn(Opcodes.ILOAD, 0);
        method.visitJumpInsn(Opcodes.IF_ICMPGE, end);
        method.visitVarInsn(Opcodes.LLOAD, 1);
        method.visitLdcInsn(1_664_525L);
        method.visitInsn(Opcodes.LMUL);
        method.visitLdcInsn(1_013_904_223L);
        method.visitInsn(Opcodes.LADD);
        method.visitVarInsn(Opcodes.ILOAD, 3);
        method.visitInsn(Opcodes.I2L);
        method.visitInsn(Opcodes.LXOR);
        method.visitVarInsn(Opcodes.LSTORE, 1);
        method.visitIincInsn(3, 1);
        method.visitJumpInsn(Opcodes.GOTO, check);
        method.visitLabel(end);
        method.visitVarInsn(Opcodes.LLOAD, 1);
        method.visitInsn(Opcodes.LRETURN);
        method.visitMaxs(0, 0);
        method.visitEnd();
        writer.visitEnd();
        return writer.toByteArray();
    }

    private static byte[] memoryClass() {
        ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        writer.visit(Opcodes.V21, Opcodes.ACC_PUBLIC, "fixture/Memory", null, "java/lang/Object", null);
        writer.visitSource("Memory.java", null);
        writer.visitField(
                Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC | Opcodes.ACC_VOLATILE,
                "published",
                "I",
                null,
                null).visitEnd();
        MethodVisitor method = writer.visitMethod(
                Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC | Opcodes.ACC_SYNCHRONIZED,
                "run",
                "([ILjava/lang/Object;)I",
                null,
                null);
        method.visitCode();
        method.visitVarInsn(Opcodes.ALOAD, 1);
        method.visitInsn(Opcodes.MONITORENTER);
        method.visitVarInsn(Opcodes.ALOAD, 0);
        method.visitInsn(Opcodes.ICONST_1);
        method.visitIntInsn(Opcodes.BIPUSH, 7);
        method.visitInsn(Opcodes.IASTORE);
        method.visitVarInsn(Opcodes.ALOAD, 0);
        method.visitInsn(Opcodes.ICONST_1);
        method.visitInsn(Opcodes.IALOAD);
        method.visitFieldInsn(Opcodes.PUTSTATIC, "fixture/Memory", "published", "I");
        method.visitVarInsn(Opcodes.ALOAD, 1);
        method.visitInsn(Opcodes.MONITOREXIT);
        method.visitFieldInsn(Opcodes.GETSTATIC, "fixture/Memory", "published", "I");
        method.visitInsn(Opcodes.IRETURN);
        method.visitMaxs(0, 0);
        method.visitEnd();
        writer.visitEnd();
        return writer.toByteArray();
    }

    private static byte[] nondeterministicClass() {
        ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        writer.visit(Opcodes.V21, Opcodes.ACC_PUBLIC, "fixture/Nondeterministic", null, "java/lang/Object", null);
        writer.visitSource("Nondeterministic.java", null);
        MethodVisitor method = writer.visitMethod(
                Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC,
                "run",
                "(Ljava/util/Random;)[Ljava/lang/Object;",
                null,
                null);
        method.visitCode();
        method.visitInsn(Opcodes.ICONST_3);
        method.visitTypeInsn(Opcodes.ANEWARRAY, "java/lang/Object");
        method.visitInsn(Opcodes.DUP);
        method.visitInsn(Opcodes.ICONST_0);
        method.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/System", "currentTimeMillis", "()J", false);
        method.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/Long", "valueOf", "(J)Ljava/lang/Long;", false);
        method.visitInsn(Opcodes.AASTORE);
        method.visitInsn(Opcodes.DUP);
        method.visitInsn(Opcodes.ICONST_1);
        method.visitMethodInsn(Opcodes.INVOKESTATIC, "java/util/UUID", "randomUUID", "()Ljava/util/UUID;", false);
        method.visitInsn(Opcodes.AASTORE);
        method.visitInsn(Opcodes.DUP);
        method.visitInsn(Opcodes.ICONST_2);
        method.visitVarInsn(Opcodes.ALOAD, 0);
        method.visitIntInsn(Opcodes.BIPUSH, 100);
        method.visitMethodInsn(Opcodes.INVOKEVIRTUAL, "java/util/Random", "nextInt", "(I)I", false);
        method.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/Integer", "valueOf", "(I)Ljava/lang/Integer;", false);
        method.visitInsn(Opcodes.AASTORE);
        method.visitInsn(Opcodes.ARETURN);
        method.visitMaxs(0, 0);
        method.visitEnd();
        writer.visitEnd();
        return writer.toByteArray();
    }

    private static byte[] fileInputClass() {
        ClassWriter writer = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        writer.visit(Opcodes.V21, Opcodes.ACC_PUBLIC, "fixture/FileInput", null, "java/lang/Object", null);
        writer.visitSource("FileInput.java", null);
        MethodVisitor method = writer.visitMethod(
                Opcodes.ACC_PUBLIC | Opcodes.ACC_STATIC,
                "run",
                "(Ljava/nio/file/Path;)Ljava/lang/String;",
                null,
                new String[] {"java/io/IOException"});
        method.visitCode();
        method.visitVarInsn(Opcodes.ALOAD, 0);
        method.visitMethodInsn(
                Opcodes.INVOKESTATIC,
                "java/nio/file/Files",
                "readString",
                "(Ljava/nio/file/Path;)Ljava/lang/String;",
                false);
        method.visitInsn(Opcodes.ARETURN);
        method.visitMaxs(0, 0);
        method.visitEnd();
        writer.visitEnd();
        return writer.toByteArray();
    }

    private static final class BytesClassLoader extends ClassLoader {
        Class<?> define(String name, byte[] bytes) {
            return defineClass(name, bytes, 0, bytes.length);
        }
    }
}
