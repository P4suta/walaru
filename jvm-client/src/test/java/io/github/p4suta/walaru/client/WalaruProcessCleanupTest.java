package io.github.p4suta.walaru.client;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.stream.Stream;
import org.junit.jupiter.api.Test;

class WalaruProcessCleanupTest {
    @Test
    void termination_reenumerates_a_tracked_child_that_spawns_a_grandchild() {
        FakeHandle root = new FakeHandle(10, null, false);
        FakeHandle child = new FakeHandle(11, root, true);
        FakeHandle lateGrandchild = new FakeHandle(12, child, false);
        root.addChild(child);
        child.spawnOnNormalTermination(lateGrandchild);

        FakeProcess process = new FakeProcess(root);

        boolean terminated = WalaruClient.terminate(process);

        assertTrue(terminated);
        assertTrue(process.completionWasAwaited);
        assertTrue(child.normalTerminationRequests > 0);
        assertTrue(lateGrandchild.normalTerminationRequests > 0);
        assertFalse(root.isAlive());
        assertFalse(child.isAlive());
        assertFalse(lateGrandchild.isAlive());
    }

    private static final class FakeProcess extends Process {
        private final FakeHandle handle;
        private boolean completionWasAwaited;

        private FakeProcess(FakeHandle handle) {
            this.handle = handle;
        }

        @Override
        public OutputStream getOutputStream() {
            return OutputStream.nullOutputStream();
        }

        @Override
        public InputStream getInputStream() {
            return InputStream.nullInputStream();
        }

        @Override
        public InputStream getErrorStream() {
            return InputStream.nullInputStream();
        }

        @Override
        public int waitFor() throws InterruptedException {
            while (handle.isAlive()) Thread.sleep(1);
            return 0;
        }

        @Override
        public boolean waitFor(long timeout, TimeUnit unit) {
            completionWasAwaited = true;
            return !handle.isAlive();
        }

        @Override
        public int exitValue() {
            if (handle.isAlive()) throw new IllegalThreadStateException("process is alive");
            return 0;
        }

        @Override
        public void destroy() {
            handle.destroy();
        }

        @Override
        public Process destroyForcibly() {
            handle.destroyForcibly();
            return this;
        }

        @Override
        public boolean isAlive() {
            return handle.isAlive();
        }

        @Override
        public long pid() {
            return handle.pid();
        }

        @Override
        public ProcessHandle toHandle() {
            return handle;
        }
    }

    private static final class FakeHandle implements ProcessHandle {
        private final long pid;
        private final FakeHandle parent;
        private final boolean survivesNormalTermination;
        private final List<FakeHandle> children = new ArrayList<>();
        private final CompletableFuture<ProcessHandle> exit = new CompletableFuture<>();
        private FakeHandle childSpawnedOnNormalTermination;
        private boolean alive = true;
        private int normalTerminationRequests;

        private FakeHandle(long pid, FakeHandle parent, boolean survivesNormalTermination) {
            this.pid = pid;
            this.parent = parent;
            this.survivesNormalTermination = survivesNormalTermination;
        }

        private void addChild(FakeHandle child) {
            children.add(child);
        }

        private void spawnOnNormalTermination(FakeHandle child) {
            childSpawnedOnNormalTermination = child;
        }

        private List<FakeHandle> visibleChildren() {
            List<FakeHandle> visible = new ArrayList<>(children);
            if (normalTerminationRequests > 0 && childSpawnedOnNormalTermination != null) {
                visible.add(childSpawnedOnNormalTermination);
            }
            return visible;
        }

        @Override
        public long pid() {
            return pid;
        }

        @Override
        public Optional<ProcessHandle> parent() {
            return Optional.ofNullable(parent).map(value -> value);
        }

        @Override
        public Stream<ProcessHandle> children() {
            return visibleChildren().stream().filter(FakeHandle::isAlive).map(value -> value);
        }

        @Override
        public Stream<ProcessHandle> descendants() {
            List<ProcessHandle> discovered = new ArrayList<>();
            for (FakeHandle child : visibleChildren()) {
                if (!child.isAlive()) continue;
                discovered.add(child);
                child.descendants().forEach(discovered::add);
            }
            return discovered.stream();
        }

        @Override
        public Info info() {
            return ProcessHandle.current().info();
        }

        @Override
        public CompletableFuture<ProcessHandle> onExit() {
            return exit;
        }

        @Override
        public boolean supportsNormalTermination() {
            return true;
        }

        @Override
        public boolean destroy() {
            normalTerminationRequests += 1;
            if (!survivesNormalTermination) markExited();
            return true;
        }

        @Override
        public boolean destroyForcibly() {
            markExited();
            return true;
        }

        @Override
        public boolean isAlive() {
            return alive;
        }

        private void markExited() {
            alive = false;
            exit.complete(this);
        }

        @Override
        public int compareTo(ProcessHandle other) {
            return Long.compare(pid, other.pid());
        }

        @Override
        public boolean equals(Object candidate) {
            return candidate instanceof ProcessHandle other && pid == other.pid();
        }

        @Override
        public int hashCode() {
            return Long.hashCode(pid);
        }
    }
}
