package io.github.p4suta.walaru.agent;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import java.util.function.Supplier;

/** Ordered deterministic-input tape used by fresh-JVM replay. */
final class InputTape implements AutoCloseable {
    private enum Mode {
        DISABLED,
        RECORD,
        REPLAY
    }

    private final Mode mode;
    private final FileChannel output;
    private final List<Entry> replay;
    private int cursor;

    private InputTape(Mode mode, FileChannel output, List<Entry> replay) {
        this.mode = mode;
        this.output = output;
        this.replay = replay;
    }

    static InputTape fromProperties() {
        String replayPath = System.getProperty("walaru.replayInputFile", "");
        String recordPath = System.getProperty("walaru.inputFile", "");
        try {
            if (!replayPath.isBlank()) return replay(Path.of(replayPath));
            if (!recordPath.isBlank()) return record(Path.of(recordPath));
            return new InputTape(Mode.DISABLED, null, List.of());
        } catch (IOException failure) {
            throw new IllegalStateException("cannot initialize deterministic input tape", failure);
        }
    }

    static InputTape disabled() {
        return new InputTape(Mode.DISABLED, null, List.of());
    }

    private static InputTape record(Path path) throws IOException {
        Path parent = path.toAbsolutePath().normalize().getParent();
        if (parent != null) Files.createDirectories(parent);
        return new InputTape(
                Mode.RECORD,
                FileChannel.open(path, StandardOpenOption.CREATE, StandardOpenOption.APPEND, StandardOpenOption.WRITE),
                List.of());
    }

    private static InputTape replay(Path path) throws IOException {
        List<Entry> entries = new ArrayList<>();
        for (String line : Files.readAllLines(path, StandardCharsets.UTF_8)) {
            if (line.isBlank()) continue;
            String[] fields = line.split("\\t", 2);
            if (fields.length != 2) throw new IOException("malformed deterministic input tape");
            entries.add(new Entry(fields[0], fields[1]));
        }
        return new InputTape(Mode.REPLAY, null, List.copyOf(entries));
    }

    synchronized String next(String kind, Supplier<String> actual) {
        String encoded;
        if (mode == Mode.REPLAY) {
            if (cursor >= replay.size()) throw new IllegalStateException("input tape ended before " + kind);
            Entry entry = replay.get(cursor++);
            if (!entry.kind.equals(kind)) {
                throw new IllegalStateException("input tape expected " + entry.kind + " but observed " + kind);
            }
            encoded = entry.encoded;
        } else {
            String value = actual.get();
            encoded = Base64.getEncoder().encodeToString(value.getBytes(StandardCharsets.UTF_8));
            if (mode == Mode.RECORD) append(kind, encoded);
        }
        AgentBridge.inputObserved(kind, encoded, decode(encoded));
        return decode(encoded);
    }

    synchronized byte[] nextBytes(String kind, ByteSupplier actual, int maxBytes) throws IOException {
        String encoded;
        if (mode == Mode.REPLAY) {
            if (cursor >= replay.size()) throw new IllegalStateException("input tape ended before " + kind);
            Entry entry = replay.get(cursor++);
            if (!entry.kind.equals(kind)) {
                throw new IllegalStateException("input tape expected " + entry.kind + " but observed " + kind);
            }
            encoded = entry.encoded;
        } else {
            byte[] value = actual.get();
            if (value.length > maxBytes) {
                AgentBridge.capabilityMissing(
                        "io",
                        "file input exceeded the explicit capture limit of " + maxBytes + " bytes");
                return value;
            }
            encoded = Base64.getEncoder().encodeToString(value);
            if (mode == Mode.RECORD) append(kind, encoded);
        }
        byte[] value = Base64.getDecoder().decode(encoded);
        AgentBridge.inputObservedSensitive(kind, value.length);
        return value;
    }

    private void append(String kind, String encoded) {
        byte[] line = (kind + '\t' + encoded + '\n').getBytes(StandardCharsets.UTF_8);
        ByteBuffer bytes = ByteBuffer.wrap(line);
        try (var lock = output.lock()) {
            if (!lock.isValid()) throw new IOException("input tape lock became invalid");
            output.position(output.size());
            while (bytes.hasRemaining()) output.write(bytes);
            output.force(false);
        } catch (IOException failure) {
            throw new IllegalStateException("cannot append deterministic input", failure);
        }
    }

    private static String decode(String encoded) {
        return new String(Base64.getDecoder().decode(encoded), StandardCharsets.UTF_8);
    }

    @Override
    public void close() throws IOException {
        if (output != null) output.close();
    }

    private record Entry(String kind, String encoded) {}

    @FunctionalInterface
    interface ByteSupplier {
        byte[] get() throws IOException;
    }
}
