package io.github.p4suta.walaru.agent;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.p4suta.walaru.Walaru;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.Executors;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class PublicApiBridgeTest {
    @TempDir
    Path directory;

    @Test
    void public_api_emits_safe_source_linked_events_and_observational_span_timing() throws Exception {
        Path events = directory.resolve("events.jsonl");
        var pool = Executors.newSingleThreadExecutor();
        pool.submit(() -> assertFalse(Walaru.active())).get();
        AgentBridge.resetForTest(events, AgentMode.FULL);
        try {
            assertFalse(Walaru.active());
            AgentBridge.testStarted("test-id", "demo.ExampleTest#works");
            assertTrue(Walaru.active());

            List<Integer> value = List.of(1, 2, 3);
            assertSame(value, Walaru.capture("result", value));
            assertEquals("secret", Walaru.captureRedacted("apiToken", "secret"));
            Walaru.captureLazy("computed", () -> 42);
            Walaru.captureLazy("broken", () -> { throw new IllegalStateException("diagnostic only"); });
            Walaru.checkpoint("partitioned", value);
            Walaru.note("decision", "kept left half");
            Walaru.run("binary search", () -> Walaru.capture("mid", 1));
            Thread.ofVirtual().start(() -> Walaru.capture("virtualThreadValue", 7)).join();
            var context = Walaru.context();
            assertTrue(context.present());
            assertEquals(11, pool.submit(context.wrapCallable(() -> Walaru.capture("pooledValue", 11))).get());

            AgentBridge.testFinished("test-id", "successful", null);
            assertFalse(context.present());
            assertFalse(pool.submit(context.wrapCallable(Walaru::active)).get());
            pool.submit(context.wrap(() -> Walaru.capture("afterTest", 99))).get();
        } finally {
            AgentBridge.closeForTest();
            pool.close();
        }

        String trace = Files.readString(events);
        assertTrue(trace.contains("\"type\":\"CAPTURE\""), trace);
        assertTrue(trace.contains("\"name\":\"computed\""), trace);
        assertTrue(trace.contains("\"value\":42"), trace);
        assertTrue(trace.contains("\"type\":\"CHECKPOINT\""), trace);
        assertTrue(trace.contains("\"type\":\"NOTE\""), trace);
        assertTrue(trace.contains("\"type\":\"SPAN_START\""), trace);
        assertTrue(trace.contains("\"type\":\"SPAN_END\""), trace);
        assertTrue(trace.contains("\"durationNanos\""), trace);
        assertTrue(trace.contains("\"virtualThread\":true"), trace);
        assertTrue(trace.contains("\"name\":\"virtualThreadValue\""), trace);
        assertTrue(trace.contains("\"name\":\"pooledValue\""), trace);
        assertFalse(trace.contains("afterTest"), trace);
        assertTrue(trace.contains("\"value\":\"<redacted>\""), trace);
        assertFalse(trace.contains("\"value\":\"secret\""), trace);
        assertTrue(trace.contains("\"path\":\"PublicApiBridgeTest.java\""), trace);
    }
}
