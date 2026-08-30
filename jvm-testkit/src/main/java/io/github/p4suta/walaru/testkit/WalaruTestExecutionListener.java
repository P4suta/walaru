package io.github.p4suta.walaru.testkit;

import io.github.p4suta.walaru.WalaruTestLifecycle;
import org.junit.platform.engine.TestExecutionResult;
import org.junit.platform.engine.support.descriptor.MethodSource;
import org.junit.platform.launcher.TestExecutionListener;
import org.junit.platform.launcher.TestIdentifier;

/** JUnit Platform listener discovered through the platform's standard ServiceLoader contract. */
public final class WalaruTestExecutionListener implements TestExecutionListener {
    @Override
    public void executionStarted(TestIdentifier identifier) {
        if (identifier.isTest() && !runnerOwnsLifecycle()) {
            WalaruTestLifecycle.started(identifier.getUniqueId(), publicName(identifier));
        }
    }

    @Override
    public void executionFinished(TestIdentifier identifier, TestExecutionResult result) {
        if (!identifier.isTest() || runnerOwnsLifecycle()) return;
        WalaruTestLifecycle.finished(
                identifier.getUniqueId(),
                result.getStatus().name().toLowerCase(java.util.Locale.ROOT),
                result.getThrowable().orElse(null));
    }

    private static boolean runnerOwnsLifecycle() {
        return Boolean.getBoolean("walaru.runnerOwnsLifecycle");
    }

    private static String publicName(TestIdentifier identifier) {
        Object source = identifier.getSource().orElse(null);
        if (source instanceof MethodSource method) {
            return method.getClassName() + "#" + method.getMethodName();
        }
        return identifier.getLegacyReportingName();
    }
}
