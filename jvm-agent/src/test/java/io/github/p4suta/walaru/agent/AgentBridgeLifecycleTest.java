package io.github.p4suta.walaru.agent;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

final class AgentBridgeLifecycleTest {
    @TempDir
    Path directory;

    @Test
    void child_test_lifecycle_does_not_deactivate_its_inherited_parent_context() throws Exception {
        Path events = directory.resolve("nested-threads.jsonl");
        AgentBridge.resetForTest(events, AgentMode.FULL);
        try {
            AgentBridge.testStarted("test-a", "demo.ParentTest#works");
            Object parentContext = AgentBridge.apiCaptureContext();
            AtomicBoolean inheritedParent = new AtomicBoolean();
            Thread child = Thread.ofVirtual().start(() -> {
                inheritedParent.set(AgentBridge.apiCaptureContext() == parentContext);
                AgentBridge.testStarted("test-b", "demo.ChildTest#works");
                AgentBridge.line("demo/Child", "run", "()V", "Child.java", 7, null, new Object[0]);
                AgentBridge.testFinished("test-b", "passed", null);
                AgentBridge.line(
                        "demo/ParentChild", "resume", "()V", "ParentChild.java", 9, null, new Object[0]);
            });
            child.join();

            assertTrue(inheritedParent.get(), "the child must exercise an inherited parent context");
            AgentBridge.line("demo/Parent", "run", "()V", "Parent.java", 11, null, new Object[0]);
            AgentBridge.testFinished("test-a", "passed", null);
        } finally {
            AgentBridge.closeForTest();
        }

        List<String> trace = Files.readAllLines(events);
        assertTrue(trace.stream().anyMatch(line -> event(line, "LINE", "test-b", "Child.java")), trace.toString());
        assertTrue(
                trace.stream().anyMatch(line -> event(line, "LINE", "test-a", "ParentChild.java")),
                trace.toString());
        assertTrue(trace.stream().anyMatch(line -> event(line, "LINE", "test-a", "Parent.java")), trace.toString());
        assertTrue(trace.stream().anyMatch(line -> event(line, "TEST_FINISH", "test-a", null)), trace.toString());
        assertTrue(trace.stream().anyMatch(line -> event(line, "TEST_FINISH", "test-b", null)), trace.toString());
    }

    @Test
    void lifecycle_id_finishes_and_deactivates_a_test_from_another_thread() throws Exception {
        Path events = directory.resolve("cross-thread-finish.jsonl");
        try (var callbacks = Executors.newSingleThreadExecutor()) {
            callbacks.submit(() -> {}).get();
            AgentBridge.resetForTest(events, AgentMode.FULL);
            try {
                AgentBridge.testStarted("test-a", "demo.AsyncTest#works");
                Object context = AgentBridge.apiCaptureContext();

                callbacks.submit(() -> AgentBridge.testFinished("test-a", "passed", null)).get();

                assertFalse(AgentBridge.apiContextActive(context));
                AgentBridge.line("demo/Async", "after", "()V", "Async.java", 19, null, new Object[0]);
            } finally {
                AgentBridge.closeForTest();
            }
        }

        List<String> trace = Files.readAllLines(events);
        assertTrue(trace.stream().anyMatch(line -> event(line, "TEST_FINISH", "test-a", null)
                && line.contains("\"testName\":\"demo.AsyncTest#works\"")), trace.toString());
        assertFalse(trace.stream().anyMatch(line -> line.contains("\"path\":\"Async.java\"")), trace.toString());
    }

    private static boolean event(String line, String type, String testId, String path) {
        return line.contains("\"type\":\"" + type + "\"")
                && line.contains("\"testId\":\"" + testId + "\"")
                && (path == null || line.contains("\"path\":\"" + path + "\""));
    }
}
