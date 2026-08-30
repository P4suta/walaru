package io.github.p4suta.walaru.client;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JavaType;
import com.fasterxml.jackson.databind.MapperFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.json.JsonMapper;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.Objects;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

/**
 * Shell-free typed client for Walaru's bounded schema-v1 CLI contract.
 *
 * <p>The client intentionally preserves exit codes 1 and 4 as data-bearing results. Only process,
 * timeout, response-bound, malformed JSON, and unsupported-schema failures throw.
 */
public final class WalaruClient {
    private static final int MAX_STDERR_BYTES = 64 * 1024;

    private final Path workspace;
    private final List<String> launcher;
    private final Duration timeout;
    private final int maxResponseBytes;
    private final ObjectMapper mapper;

    private WalaruClient(Builder builder) {
        workspace = builder.workspace.toAbsolutePath().normalize();
        launcher = List.copyOf(builder.launcher);
        timeout = builder.timeout;
        maxResponseBytes = builder.maxResponseBytes;
        mapper = JsonMapper.builder()
                .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)
                .enable(DeserializationFeature.READ_UNKNOWN_ENUM_VALUES_USING_DEFAULT_VALUE)
                .enable(MapperFeature.ACCEPT_CASE_INSENSITIVE_ENUMS)
                .build();
    }

    public static Builder builder(Path workspace) {
        return new Builder(workspace);
    }

    public WalaruResult<WalaruData.Status> status() {
        return invoke(List.of("status"), WalaruQuery.defaults(), WalaruData.Status.class);
    }

    public WalaruResult<com.fasterxml.jackson.databind.JsonNode> doctor() {
        return invoke(List.of("doctor"), WalaruQuery.defaults(), com.fasterxml.jackson.databind.JsonNode.class);
    }

    public WalaruResult<WalaruData.Tests> tests() {
        return tests(WalaruQuery.defaults());
    }

    public WalaruResult<WalaruData.Tests> tests(WalaruQuery query) {
        return invoke(List.of("tests"), query, WalaruData.Tests.class);
    }

    public WalaruResult<WalaruData.FailureResponse> failure(String failureId) {
        return invoke(
                List.of("failure", required(failureId, "failureId")),
                WalaruQuery.defaults(),
                WalaruData.FailureResponse.class);
    }

    public WalaruResult<WalaruData.Verify> verify() {
        return verify(VerifyOptions.impacted());
    }

    public WalaruResult<WalaruData.Verify> verify(VerifyOptions options) {
        Objects.requireNonNull(options, "options");
        List<String> command = new ArrayList<>();
        command.add("verify");
        if (options.full()) command.add("--full");
        if (options.since() != null && !options.since().isBlank()) {
            command.add("--since");
            command.add(options.since());
        }
        return invoke(command, WalaruQuery.defaults(), WalaruData.Verify.class);
    }

    public WalaruResult<WalaruData.Explain> explain() {
        return explain(VerifyOptions.impacted(), 5);
    }

    public WalaruResult<WalaruData.Explain> explain(VerifyOptions options, int maxFailures) {
        Objects.requireNonNull(options, "options");
        if (maxFailures < 1 || maxFailures > 20) {
            throw new IllegalArgumentException("maxFailures must be between 1 and 20");
        }
        List<String> command = new ArrayList<>();
        command.add("explain");
        if (options.full()) command.add("--full");
        if (options.since() != null && !options.since().isBlank()) {
            command.add("--since");
            command.add(options.since());
        }
        command.add("--max-failures");
        command.add(Integer.toString(maxFailures));
        return invoke(command, WalaruQuery.defaults(), WalaruData.Explain.class);
    }

    public WalaruResult<WalaruData.Impact> impact(String subject) {
        return invoke(
                List.of("impact", required(subject, "subject")),
                WalaruQuery.defaults(),
                WalaruData.Impact.class);
    }

    public WalaruResult<WalaruData.Coverage> coverage(String subject, WalaruQuery query) {
        return invoke(
                List.of("coverage", required(subject, "subject")),
                query,
                WalaruData.Coverage.class);
    }

    public WalaruResult<WalaruData.Trace> trace(String runOrTest, WalaruQuery query) {
        return invoke(
                List.of("trace", required(runOrTest, "runOrTest")),
                query,
                WalaruData.Trace.class);
    }

    public WalaruResult<WalaruData.Values> values(String eventId) {
        return invoke(
                List.of("values", required(eventId, "eventId")),
                WalaruQuery.defaults(),
                WalaruData.Values.class);
    }

    public WalaruResult<WalaruData.Recording> record(String testId) {
        return record(testId, false);
    }

    public WalaruResult<WalaruData.Recording> record(String testId, boolean captureFileIo) {
        List<String> command = new ArrayList<>(List.of("record", required(testId, "testId")));
        if (captureFileIo) command.add("--capture-file-io");
        return invoke(command, WalaruQuery.defaults(), WalaruData.Recording.class);
    }

    public WalaruResult<WalaruData.Replay> replay(String recordingId, String eventId) {
        WalaruQuery query = new WalaruQuery(100, null, 65_536, required(eventId, "eventId"), List.of());
        return invoke(
                List.of("replay", required(recordingId, "recordingId")),
                query,
                WalaruData.Replay.class);
    }

    public WalaruResult<WalaruData.Replay> reverse(ReverseRequest request) {
        Objects.requireNonNull(request, "request");
        List<String> command = new ArrayList<>(List.of(
                "reverse", request.recordingId(), "--from", request.fromEventId()));
        if (request.step() != null) {
            command.add("--step");
            command.add(request.step().name().toLowerCase(Locale.ROOT));
        } else {
            command.add("--until");
            command.add(request.until());
        }
        if (request.watch() != null) {
            command.add("--watch");
            command.add(request.watch());
        }
        return invoke(command, WalaruQuery.defaults(), WalaruData.Replay.class);
    }

    public WalaruResult<com.fasterxml.jackson.databind.JsonNode> stop() {
        return invoke(List.of("stop"), WalaruQuery.defaults(), com.fasterxml.jackson.databind.JsonNode.class);
    }

    private <T> WalaruResult<T> invoke(
            List<String> command, WalaruQuery query, Class<T> dataType) {
        Objects.requireNonNull(query, "query");
        List<String> arguments = new ArrayList<>(launcher);
        arguments.add("--workspace");
        arguments.add(workspace.toString());
        arguments.add("--format");
        arguments.add("json");
        arguments.add("--limit");
        arguments.add(Integer.toString(query.limit()));
        arguments.add("--max-bytes");
        arguments.add(Integer.toString(query.maxBytes()));
        if (query.cursor() != null && !query.cursor().isBlank()) {
            arguments.add("--cursor");
            arguments.add(query.cursor());
        }
        if (query.at() != null && !query.at().isBlank()) {
            arguments.add("--at");
            arguments.add(query.at());
        }
        if (!query.fields().isEmpty()) {
            arguments.add("--fields");
            arguments.add(String.join(",", query.fields()));
        }
        arguments.addAll(command);
        return execute(arguments, dataType);
    }

    private <T> WalaruResult<T> execute(List<String> arguments, Class<T> dataType) {
        Process process;
        try {
            process = new ProcessBuilder(arguments)
                    .directory(workspace.toFile())
                    .redirectInput(ProcessBuilder.Redirect.PIPE)
                    .start();
            process.getOutputStream().close();
        } catch (IOException failure) {
            throw new WalaruClientException("cannot start Walaru: " + failure.getMessage(), failure);
        }

        ExecutorService readers = Executors.newVirtualThreadPerTaskExecutor();
        Future<BoundedBytes> stdout =
                readers.submit(() -> readBoundedLine(process.getInputStream(), maxResponseBytes));
        Future<BoundedBytes> stderr =
                readers.submit(() -> readBoundedLine(process.getErrorStream(), MAX_STDERR_BYTES));
        long deadline = System.nanoTime() + timeout.toNanos();
        try {
            boolean completed = process.waitFor(timeout.toNanos(), TimeUnit.NANOSECONDS);
            if (!completed) {
                terminate(process);
                throw new WalaruClientException("Walaru exceeded timeout " + timeout);
            }
            BoundedBytes output = future(stdout, "stdout", deadline);
            int exitCode = process.exitValue();
            if (output.exceeded()) {
                throw new WalaruClientException(
                        "Walaru response exceeded the client bound of " + maxResponseBytes + " bytes",
                        exitCode);
            }
            if (output.bytes().length == 0) {
                BoundedBytes errors = future(stderr, "stderr", deadline);
                throw new WalaruClientException(
                        "Walaru returned no JSON (exit " + exitCode + "): " + errors.text(), exitCode);
            }
            JavaType envelopeType = mapper.getTypeFactory()
                    .constructParametricType(WalaruEnvelope.class, mapper.constructType(dataType));
            WalaruEnvelope<T> envelope;
            try {
                envelope = mapper.readValue(output.bytes(), envelopeType);
            } catch (IOException malformed) {
                BoundedBytes errors = future(stderr, "stderr", deadline);
                throw new WalaruClientException(
                        "Walaru returned malformed JSON (exit " + exitCode + "): " + errors.text(),
                        malformed);
            }
            if (!"1".equals(envelope.schemaVersion())) {
                throw new WalaruClientException(
                        "unsupported Walaru schema version " + envelope.schemaVersion(), exitCode);
            }
            return new WalaruResult<>(WalaruExit.fromCode(exitCode), exitCode, envelope);
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            terminate(process);
            throw new WalaruClientException("interrupted while waiting for Walaru", interrupted);
        } finally {
            stdout.cancel(true);
            stderr.cancel(true);
            readers.shutdownNow();
        }
    }

    private static BoundedBytes future(Future<BoundedBytes> future, String stream, long deadline) {
        try {
            long remaining = Math.max(1L, deadline - System.nanoTime());
            return future.get(remaining, TimeUnit.NANOSECONDS);
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            throw new WalaruClientException("interrupted while reading Walaru " + stream, interrupted);
        } catch (TimeoutException timeout) {
            throw new WalaruClientException("timeout while reading Walaru " + stream, timeout);
        } catch (ExecutionException failure) {
            throw new WalaruClientException(
                    "cannot read Walaru " + stream + ": " + failure.getCause().getMessage(),
                    failure.getCause());
        }
    }

    private static BoundedBytes readBoundedLine(InputStream input, int maximum) throws IOException {
        ByteArrayOutputStream kept = new ByteArrayOutputStream(Math.min(maximum, 8 * 1024));
        byte[] chunk = new byte[8 * 1024];
        long total = 0;
        int read;
        while ((read = input.read(chunk)) != -1) {
            for (int index = 0; index < read; index += 1) {
                if (chunk[index] == '\n') return new BoundedBytes(kept.toByteArray(), total > maximum);
                if (total < maximum) kept.write(chunk[index]);
                total += 1;
            }
        }
        return new BoundedBytes(kept.toByteArray(), total > maximum);
    }

    private static void terminate(Process process) {
        List<ProcessHandle> descendants = new ArrayList<>(process.descendants().toList());
        Collections.reverse(descendants);
        descendants.forEach(ProcessHandle::destroy);
        process.destroy();
        try {
            if (!process.waitFor(250, TimeUnit.MILLISECONDS)) {
                descendants.stream().filter(ProcessHandle::isAlive).forEach(ProcessHandle::destroyForcibly);
                process.destroyForcibly();
            }
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            process.destroyForcibly();
        }
    }

    private static String required(String value, String name) {
        if (value == null || value.isBlank()) throw new IllegalArgumentException(name + " is blank");
        return value;
    }

    private record BoundedBytes(byte[] bytes, boolean exceeded) {
        String text() {
            return new String(bytes, java.nio.charset.StandardCharsets.UTF_8).strip();
        }
    }

    public static final class Builder {
        private final Path workspace;
        private List<String> launcher;
        private Duration timeout = Duration.ofMinutes(5);
        private int maxResponseBytes = 1024 * 1024;

        private Builder(Path workspace) {
            this.workspace = Objects.requireNonNull(workspace, "workspace");
            String configured = System.getProperty("walaru.binary");
            if (configured == null || configured.isBlank()) configured = System.getenv("WALARU_BINARY");
            launcher = List.of(configured == null || configured.isBlank() ? "walaru" : configured);
        }

        /** Selects one Walaru executable path. */
        public Builder binary(Path binary) {
            launcher = List.of(Objects.requireNonNull(binary, "binary").toString());
            return this;
        }

        /**
         * Selects a shell-free executable prefix. This supports wrappers and hermetic test runners.
         */
        public Builder launcher(List<String> executableAndArguments) {
            Objects.requireNonNull(executableAndArguments, "executableAndArguments");
            if (executableAndArguments.isEmpty()
                    || executableAndArguments.stream().anyMatch(value -> value == null || value.isBlank())) {
                throw new IllegalArgumentException("launcher must contain non-blank arguments");
            }
            launcher = List.copyOf(executableAndArguments);
            return this;
        }

        public Builder timeout(Duration requested) {
            Objects.requireNonNull(requested, "requested");
            if (requested.isZero()
                    || requested.isNegative()
                    || requested.compareTo(Duration.ofHours(24)) > 0) {
                throw new IllegalArgumentException("timeout must be positive and at most 24 hours");
            }
            timeout = requested;
            return this;
        }

        public Builder maxResponseBytes(int requested) {
            if (requested < 4_096 || requested > 16 * 1024 * 1024) {
                throw new IllegalArgumentException("maxResponseBytes must be between 4096 and 16777216");
            }
            maxResponseBytes = requested;
            return this;
        }

        public WalaruClient build() {
            if (!Files.isDirectory(workspace)) {
                throw new IllegalArgumentException("workspace is not a directory: " + workspace);
            }
            return new WalaruClient(this);
        }
    }
}
