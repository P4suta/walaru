package io.github.p4suta.walaru.agent;

import org.testng.ITestListener;
import org.testng.ITestResult;

/** TestNG lifecycle bridge loaded from the agent jar without target-repository changes. */
public final class WalaruTestNgListener implements ITestListener {
    @Override
    public void onTestStart(ITestResult result) {
        if (runnerOwnsLifecycle()) return;
        String name = publicName(result);
        AgentBridge.testStarted(name, name);
    }

    @Override
    public void onTestSuccess(ITestResult result) {
        finish(result, "successful", null);
    }

    @Override
    public void onTestFailure(ITestResult result) {
        finish(result, "failed", result.getThrowable());
    }

    @Override
    public void onTestSkipped(ITestResult result) {
        finish(result, "aborted", result.getThrowable());
    }

    private static void finish(ITestResult result, String status, Throwable failure) {
        if (runnerOwnsLifecycle()) return;
        AgentBridge.testFinished(publicName(result), status, failure);
    }

    private static boolean runnerOwnsLifecycle() {
        return Boolean.getBoolean("walaru.runnerOwnsLifecycle");
    }

    private static String publicName(ITestResult result) {
        return result.getTestClass().getRealClass().getName() + "#" + result.getMethod().getMethodName();
    }
}
