package io.github.p4suta.walaru.agent;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;

/** Cooperative event-boundary scheduler for deterministic multi-thread replay. */
final class ReplayScheduler {
    private static final long WAIT_NANOS = Duration.ofSeconds(10).toNanos();

    private final List<Entry> expected;
    private int cursor;
    private boolean writing;

    private ReplayScheduler(List<Entry> expected) {
        this.expected = expected;
    }

    static ReplayScheduler fromProperties() {
        String path = System.getProperty("walaru.replayScheduleFile", "");
        if (path.isBlank()) return disabled();
        try {
            List<Entry> entries = new ArrayList<>();
            for (String line : Files.readAllLines(Path.of(path), StandardCharsets.UTF_8)) {
                if (line.isBlank()) continue;
                String[] fields = line.split("\\t", 2);
                if (fields.length != 2) throw new IOException("malformed replay schedule");
                String threadKey = new String(HexFormat.of().parseHex(fields[1]), StandardCharsets.UTF_8);
                entries.add(new Entry(fields[0], threadKey));
            }
            return new ReplayScheduler(List.copyOf(entries));
        } catch (IOException | IllegalArgumentException failure) {
            throw new IllegalStateException("cannot initialize replay schedule", failure);
        }
    }

    static ReplayScheduler disabled() {
        return new ReplayScheduler(List.of());
    }

    static String threadKey() {
        Thread thread = Thread.currentThread();
        String name = thread.getName();
        return (thread.isVirtual() ? "virtual:" : "platform:") + (name.isBlank() ? "<unnamed>" : name);
    }

    synchronized void before(String rawType, String threadKey) {
        if (expected.isEmpty()) return;
        String category = category(rawType);
        long deadline = System.nanoTime() + WAIT_NANOS;
        while (cursor < expected.size()
                && (writing || !expected.get(cursor).threadKey.equals(threadKey))) {
            long remaining = deadline - System.nanoTime();
            if (remaining <= 0) {
                throw new IllegalStateException(
                        "replay schedule timed out waiting for " + expected.get(cursor).threadKey);
            }
            try {
                long millis = Math.max(1, remaining / 1_000_000L);
                wait(millis);
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt();
                throw new IllegalStateException("replay schedule wait was interrupted", interrupted);
            }
        }
        if (cursor >= expected.size()) throw new IllegalStateException("fresh execution exceeded replay schedule");
        Entry entry = expected.get(cursor);
        if (!entry.category.equals(category)) {
            throw new IllegalStateException(
                    "replay schedule expected " + entry.category + " but observed " + category);
        }
        writing = true;
    }

    synchronized void after() {
        if (expected.isEmpty()) return;
        cursor++;
        writing = false;
        notifyAll();
    }

    private static String category(String type) {
        return switch (type) {
            case "METHOD_ENTER", "METHOD_EXIT", "CALL" -> "CALL";
            default -> type;
        };
    }

    private record Entry(String category, String threadKey) {}
}
