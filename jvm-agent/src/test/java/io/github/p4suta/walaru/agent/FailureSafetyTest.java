package io.github.p4suta.walaru.agent;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

final class FailureSafetyTest {
    @TempDir
    Path directory;

    @Test
    void capturesTrustedFailureReasonWithoutInvokingCustomThrowableMessage() throws Exception {
        Path events = directory.resolve("failures.jsonl");
        AgentBridge.resetForTest(events, AgentMode.FULL);
        HostileFailure hostile = new HostileFailure();
        AgentBridge.testStarted("hostile", "demo.HostileTest#fails");
        AgentBridge.testFinished("hostile", "failed", hostile);
        AgentBridge.testStarted("assertion", "demo.AssertionTest#fails");
        AgentBridge.testFinished("assertion", "failed", new AssertionError("expected secretToken=abc"));
        AgentBridge.closeForTest();

        assertEquals(0, hostile.messageCalls);
        String trace = Files.readString(events);
        assertTrue(trace.contains("message unavailable without invoking user code"), trace);
        assertTrue(trace.contains("expected secretToken=&lt;redacted&gt;")
                || trace.contains("expected secretToken=<redacted>"), trace);
        assertTrue(trace.contains("\"frames\":["), trace);
    }

    private static final class HostileFailure extends RuntimeException {
        private static final long serialVersionUID = 1L;
        private int messageCalls;

        @Override
        public String getMessage() {
            messageCalls++;
            return "must-not-run";
        }
    }
}
