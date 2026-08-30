package io.github.p4suta.walaru.client;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.FileSystemException;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.time.Duration;
import java.time.Instant;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class WalaruClientTest {
    @TempDir
    Path workspace;

    @Test
    void parses_typed_pages_and_preserves_machine_safe_next_cursor() {
        WalaruClient client = client("normal").build();
        WalaruQuery query = new WalaruQuery(7, null, 8_192, null, List.of("tests.id", "tests.module"));

        WalaruResult<WalaruData.Tests> result = client.tests(query);

        assertTrue(result.succeeded());
        assertEquals("demo.SearchTest#finds", result.envelope().data().tests().getFirst().id());
        assertEquals("cursor-7", result.envelope().page().nextCursor());
        assertEquals(7, result.envelope().page().limit());
    }

    @Test
    void test_failure_is_a_typed_result_instead_of_an_exception() {
        WalaruResult<WalaruData.Verify> result = client("failure").build().verify();

        assertEquals(WalaruExit.TEST_FAILURE, result.exit());
        assertFalse(result.succeeded());
        assertTrue(result.hasUsableData());
        assertEquals(List.of("failure-1"), result.envelope().data().failures());
    }

    @Test
    void explain_is_one_typed_operation_and_keeps_failure_evidence_usable() {
        WalaruResult<WalaruData.Explain> result =
                client("explainFailure").build().explain(VerifyOptions.fullWorkspace(), 3);

        assertEquals(WalaruExit.TEST_FAILURE, result.exit());
        assertTrue(result.hasUsableData());
        assertEquals("failed", result.envelope().data().verification().status());
        WalaruData.Explanation explanation = result.envelope().data().explanations().getFirst();
        assertEquals("demo.SearchTest#fails", explanation.failure().testId());
        assertEquals(18, explanation.analysis().focus().line());
        assertEquals("<redacted>", explanation.analysis().evidence().getFirst().value().textValue());
        assertEquals("rec-1", explanation.recording().id());
        assertFalse(result.envelope().data().recordingBudgetExhausted());
    }

    @Test
    void reverse_arguments_are_shell_free_and_validated_before_launch() {
        WalaruResult<WalaruData.Replay> result = client("normal")
                .build()
                .reverse(ReverseRequest.watch("rec-1", "evt-9", "demo.Counter#value"));

        assertTrue(result.envelope().data().verified());
        assertEquals("write", result.envelope().data().event().kind());
        assertThrows(
                IllegalArgumentException.class,
                () -> new ReverseRequest("rec-1", "evt-9", ReverseRequest.Step.LINE, null, "field"));
    }

    @Test
    void response_size_and_process_time_are_hard_bounded() throws Exception {
        WalaruClient oversized = client("oversized").maxResponseBytes(4_096).build();
        WalaruClientException size = assertThrows(WalaruClientException.class, oversized::status);
        assertTrue(size.getMessage().contains("exceeded"));

        Path timeoutWorkspace = Files.createDirectory(workspace.resolve("timeout-workspace"));
        WalaruClient slow = client(timeoutWorkspace, "slow").timeout(Duration.ofMillis(50)).build();
        WalaruClientException timeout = assertThrows(WalaruClientException.class, slow::status);
        assertTrue(timeout.getMessage().contains("timeout"));
        Files.delete(timeoutWorkspace);
        assertFalse(Files.exists(timeoutWorkspace));
        assertThrows(
                IllegalArgumentException.class,
                () -> client("normal").timeout(Duration.ofDays(2)));
    }

    @Test
    void one_line_response_does_not_wait_for_a_daemon_that_inherited_process_pipes() {
        WalaruClient client = client("lingering").timeout(Duration.ofSeconds(4)).build();
        Instant started = Instant.now();

        WalaruResult<WalaruData.Status> result = client.status();

        assertTrue(result.succeeded());
        assertTrue(Duration.between(started, Instant.now()).compareTo(Duration.ofSeconds(4)) < 0);
    }

    @Test
    void interruption_is_preserved_after_the_process_releases_its_workspace() throws Exception {
        Path interruptedWorkspace = Files.createDirectory(workspace.resolve("interrupted-workspace"));
        Path started = interruptedWorkspace.resolve("started");
        WalaruClient client = client(interruptedWorkspace, "interruptible")
                .timeout(Duration.ofSeconds(10))
                .build();
        AtomicReference<WalaruClientException> failure = new AtomicReference<>();
        AtomicBoolean interruptPreserved = new AtomicBoolean();
        Thread caller = Thread.ofPlatform().unstarted(() -> {
            try {
                client.status();
            } catch (WalaruClientException expected) {
                failure.set(expected);
                interruptPreserved.set(Thread.currentThread().isInterrupted());
            }
        });

        caller.start();
        try {
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
            while (!Files.exists(started) && caller.isAlive() && System.nanoTime() < deadline) {
                Thread.sleep(10);
            }
            assertTrue(Files.exists(started));
            ProcessHandle fixture = ProcessHandle.of(Long.parseLong(Files.readString(started)))
                    .orElseThrow(() -> new AssertionError("fixture process exited before interruption"));
            assertTrue(fixture.isAlive());

            caller.interrupt();
            caller.join(TimeUnit.SECONDS.toMillis(10));

            assertFalse(caller.isAlive());
            assertNotNull(failure.get());
            assertTrue(failure.get().getMessage().contains("interrupted"));
            assertTrue(interruptPreserved.get());
            assertFalse(fixture.isAlive());
            Files.delete(started);
            deleteEventually(interruptedWorkspace, Duration.ofSeconds(2));
        } finally {
            if (caller.isAlive()) {
                caller.interrupt();
                caller.join(TimeUnit.SECONDS.toMillis(10));
            }
        }
    }

    private static void deleteEventually(Path path, Duration timeout) throws Exception {
        long deadline = System.nanoTime() + timeout.toNanos();
        while (true) {
            try {
                Files.delete(path);
                return;
            } catch (FileSystemException busy) {
                if (System.nanoTime() >= deadline) throw busy;
                Thread.sleep(10);
            }
        }
    }

    private WalaruClient.Builder client(String mode) {
        return client(workspace, mode);
    }

    private WalaruClient.Builder client(Path workingDirectory, String mode) {
        String executable = System.getProperty("os.name").toLowerCase().contains("win") ? "java.exe" : "java";
        Path java = Path.of(System.getProperty("java.home"), "bin", executable);
        return WalaruClient.builder(workingDirectory).launcher(List.of(
                java.toString(),
                "-cp",
                System.getProperty("java.class.path"),
                FixtureCli.class.getName(),
                mode));
    }

    public static final class FixtureCli {
        private FixtureCli() {}

        public static void main(String[] raw) throws Exception {
            String mode = raw[0];
            List<String> arguments = Arrays.asList(raw).subList(1, raw.length);
            if (mode.equals("slow")) {
                Thread.sleep(10_000);
                return;
            }
            if (mode.equals("interruptible")) {
                int workspaceArgument = arguments.indexOf("--workspace");
                Path fixtureWorkspace = Path.of(arguments.get(workspaceArgument + 1));
                Path started = fixtureWorkspace.resolve("started");
                Path pending = fixtureWorkspace.resolve("started.pending");
                Files.writeString(pending, Long.toString(ProcessHandle.current().pid()));
                Files.move(pending, started, StandardCopyOption.ATOMIC_MOVE);
                Thread.sleep(10_000);
                return;
            }
            if (mode.equals("oversized")) {
                System.out.print(envelope("ok", "{\"padding\":\"" + "x".repeat(8_000) + "\"}", "null"));
                return;
            }
            if (mode.equals("lingeringChild")) {
                Thread.sleep(6_000);
                return;
            }
            if (mode.equals("lingering")) {
                String executable = System.getProperty("os.name").toLowerCase().contains("win")
                        ? "java.exe"
                        : "java";
                Path java = Path.of(System.getProperty("java.home"), "bin", executable);
                new ProcessBuilder(
                                java.toString(),
                                "-cp",
                                System.getProperty("java.class.path"),
                                FixtureCli.class.getName(),
                                "lingeringChild")
                        .directory(Path.of(System.getProperty("java.io.tmpdir")).toFile())
                        .inheritIO()
                        .start();
                System.out.print(envelope(
                        "ok",
                        "{\"running\":true,\"pid\":1,\"version\":\"0.1.0\","
                                + "\"stateDirectory\":\"state\",\"database\":\"db\",\"socket\":\"socket\"}",
                        "null"));
                return;
            }
            if (mode.equals("failure")) {
                System.out.print(envelope(
                        "failure",
                        "{\"runId\":\"run-1\",\"revision\":\"rev-1\",\"status\":\"failed\","
                                + "\"exitCode\":1,\"tests\":[\"demo.SearchTest#fails\"],"
                                + "\"failures\":[\"failure-1\"],\"events\":12,\"cached\":false,"
                                + "\"selection\":\"impact\",\"selectedTests\":[],\"requeued\":false}",
                        "null"));
                System.exit(1);
            }

            if (mode.equals("explainFailure")) {
                require(arguments.containsAll(List.of("explain", "--full", "--max-failures", "3")));
                System.out.print(envelope(
                        "failure",
                        "{\"verification\":{\"runId\":\"run-1\",\"revision\":\"rev-1\","
                                + "\"status\":\"failed\",\"exitCode\":1,\"tests\":[\"demo.SearchTest#fails\"],"
                                + "\"failures\":[\"failure-1\"],\"events\":8,\"cached\":false,"
                                + "\"selection\":\"full\",\"selectedTests\":[],\"requeued\":false},"
                                + "\"explanations\":[{\"failure\":{\"id\":\"failure-1\","
                                + "\"runId\":\"run-1\",\"testId\":\"demo.SearchTest#fails\","
                                + "\"exceptionType\":\"java.lang.AssertionError\",\"message\":\"boom\","
                                + "\"eventId\":\"evt-8\",\"frames\":[]},\"analysis\":{"
                                + "\"summary\":\"Assertion failed\",\"likelyCause\":\"captured token\","
                                + "\"focus\":{\"path\":\"Search.java\",\"line\":18,\"column\":1,"
                                + "\"symbol\":\"demo.Search#find\"},\"evidence\":[{\"kind\":\"capture\","
                                + "\"label\":\"Captured token\",\"eventId\":\"evt-7\",\"location\":null,"
                                + "\"value\":\"<redacted>\"}],\"suggestions\":[\"Inspect the focused line.\"]},"
                                + "\"recording\":{\"id\":\"rec-1\",\"testId\":\"demo.SearchTest#fails\","
                                + "\"revision\":\"rev-1\",\"events\":8,\"capabilities\":{\"backend\":\"jvm\","
                                + "\"completeness\":\"complete\",\"supported\":[],\"unavailable\":{}}}}],"
                                + "\"omittedFailures\":0,\"recordingBudgetExhausted\":false}",
                        "null"));
                System.exit(1);
            }

            String command = command(arguments);
            switch (command) {
                case "tests" -> {
                    require(arguments.containsAll(List.of("--limit", "7", "--fields", "tests.id,tests.module")));
                    System.out.print(envelope(
                            "ok",
                            "{\"tests\":[{\"id\":\"demo.SearchTest#finds\","
                                    + "\"displayName\":\"finds\",\"module\":\":\","
                                    + "\"lastStatus\":\"passed\",\"lastFailureId\":null}]}",
                            "{\"cursor\":null,\"nextCursor\":\"cursor-7\",\"limit\":7,\"returned\":1}"));
                }
                case "reverse" -> {
                    require(arguments.containsAll(List.of(
                            "reverse", "rec-1", "--from", "evt-9", "--step", "write", "--watch", "demo.Counter#value")));
                    System.out.print(envelope(
                            "ok",
                            "{\"recordingId\":\"rec-1\",\"replayRunId\":\"run-2\","
                                    + "\"backend\":\"jvm\",\"verified\":true,\"event\":{"
                                    + "\"id\":\"evt-8\",\"sequence\":8,\"threadId\":1,"
                                    + "\"threadKey\":\"platform:test\",\"virtualThread\":false,"
                                    + "\"coroutine\":false,\"kind\":\"write\",\"location\":null,"
                                    + "\"values\":{},\"observations\":{},\"stateHash\":\"state\","
                                    + "\"outputIndex\":0}}",
                            "null"));
                }
                case "status" -> System.out.print(envelope(
                        "ok",
                        "{\"running\":true,\"pid\":1,\"version\":\"0.1.0\","
                                + "\"stateDirectory\":\"state\",\"database\":\"db\",\"socket\":\"socket\"}",
                        "null"));
                default -> throw new IllegalArgumentException("unexpected command: " + arguments);
            }
        }

        private static String command(List<String> arguments) {
            for (String candidate : List.of("status", "tests", "reverse", "explain")) {
                if (arguments.contains(candidate)) return candidate;
            }
            return "unknown";
        }

        private static void require(boolean condition) {
            if (!condition) System.exit(2);
        }

        private static String envelope(String status, String data, String page) {
            return "{\"schemaVersion\":\"1\",\"workspaceId\":\"ws-test\","
                    + "\"revision\":\"rev-test\",\"sessionId\":\"session-test\","
                    + "\"runId\":null,\"status\":\"" + status + "\",\"data\":" + data + ","
                    + "\"diagnostics\":[],\"capabilities\":{\"backend\":\"none\","
                    + "\"completeness\":\"unsupported\",\"supported\":[],\"unavailable\":{}},"
                    + "\"nextActions\":[],\"page\":" + page + "}";
        }
    }
}
