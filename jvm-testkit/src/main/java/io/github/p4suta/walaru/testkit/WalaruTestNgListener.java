package io.github.p4suta.walaru.testkit;

import io.github.p4suta.walaru.WalaruTestLifecycle;
import org.testng.ITestListener;
import org.testng.ITestResult;

/** TestNG listener discovered through TestNG's standard ServiceLoader contract. */
public final class WalaruTestNgListener implements ITestListener {
    @Override
    public void onTestStart(ITestResult result) {
        if (runnerOwnsLifecycle()) return;
        String name = publicName(result);
        WalaruTestLifecycle.started(name, name);
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
        WalaruTestLifecycle.finished(publicName(result), status, failure);
    }

    private static boolean runnerOwnsLifecycle() {
        return Boolean.getBoolean("walaru.runnerOwnsLifecycle");
    }

    private static String publicName(ITestResult result) {
        return result.getTestClass().getRealClass().getName() + "#" + result.getMethod().getMethodName();
    }
}
