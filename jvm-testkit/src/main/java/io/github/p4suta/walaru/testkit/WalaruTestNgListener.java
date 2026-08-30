package io.github.p4suta.walaru.testkit;

import io.github.p4suta.walaru.WalaruTestLifecycle;
import org.testng.ITestListener;
import org.testng.ITestResult;

/** TestNG listener discovered through TestNG's standard ServiceLoader contract. */
public final class WalaruTestNgListener implements ITestListener {
    private static final String LIFECYCLE_ID_ATTRIBUTE =
            WalaruTestNgListener.class.getName() + ".lifecycleId";

    @Override
    public void onTestStart(ITestResult result) {
        if (runnerOwnsLifecycle()) return;
        String name = publicName(result);
        String lifecycleId = lifecycleId(result, name);
        result.setAttribute(LIFECYCLE_ID_ATTRIBUTE, lifecycleId);
        WalaruTestLifecycle.started(lifecycleId, name);
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
        String name = publicName(result);
        Object remembered = result.getAttribute(LIFECYCLE_ID_ATTRIBUTE);
        String lifecycleId = remembered instanceof String id ? id : lifecycleId(result, name);
        WalaruTestLifecycle.finished(lifecycleId, status, failure);
    }

    private static boolean runnerOwnsLifecycle() {
        return Boolean.getBoolean("walaru.runnerOwnsLifecycle");
    }

    private static String publicName(ITestResult result) {
        return result.getTestClass().getRealClass().getName() + "#" + result.getMethod().getMethodName();
    }

    private static String lifecycleId(ITestResult result, String publicName) {
        String invocationId = result.id();
        if (invocationId == null || invocationId.isBlank()) {
            invocationId = Long.toUnsignedString(result.getStartMillis())
                    + '-'
                    + Integer.toUnsignedString(System.identityHashCode(result));
        }
        return publicName + "::testng:" + invocationId;
    }
}
