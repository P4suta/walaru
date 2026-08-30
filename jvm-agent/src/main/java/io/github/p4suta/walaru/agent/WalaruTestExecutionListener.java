package io.github.p4suta.walaru.agent;

import org.junit.platform.engine.TestExecutionResult;
import org.junit.platform.engine.support.descriptor.MethodSource;
import org.junit.platform.launcher.TestExecutionListener;
import org.junit.platform.launcher.TestIdentifier;

/** JUnit Platform bridge; loaded from the agent jar on the test runtime classpath. */
public final class WalaruTestExecutionListener implements TestExecutionListener {
    @Override
    public void executionStarted(TestIdentifier identifier) {
        if (identifier.isTest() && !Boolean.getBoolean("walaru.runnerOwnsLifecycle")) {
            AgentBridge.testStarted(identifier.getUniqueId(), publicName(identifier));
        }
    }

    @Override
    public void executionFinished(TestIdentifier identifier, TestExecutionResult result) {
        if (!identifier.isTest() || Boolean.getBoolean("walaru.runnerOwnsLifecycle")) return;
        AgentBridge.testFinished(
                identifier.getUniqueId(), result.getStatus().name().toLowerCase(), result.getThrowable().orElse(null));
    }

    private static String publicName(TestIdentifier identifier) {
        Object source = identifier.getSource().orElse(null);
        if (source instanceof MethodSource method) {
            return method.getClassName() + "#" + method.getMethodName();
        }
        return identifier.getLegacyReportingName();
    }
}
