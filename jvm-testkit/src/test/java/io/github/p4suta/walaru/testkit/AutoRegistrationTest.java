package io.github.p4suta.walaru.testkit;

import static org.junit.jupiter.api.Assertions.assertDoesNotThrow;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.p4suta.walaru.WalaruTestLifecycle;
import java.util.ServiceLoader;
import org.junit.jupiter.api.Test;
import org.junit.platform.launcher.TestExecutionListener;
import org.testng.ITestNGListener;

class AutoRegistrationTest {
    @Test
    void junit_platform_and_testng_discover_the_adapters_without_configuration() {
        assertTrue(ServiceLoader.load(TestExecutionListener.class).stream()
                .anyMatch(provider -> provider.type() == WalaruTestExecutionListener.class));
        assertTrue(ServiceLoader.load(ITestNGListener.class).stream()
                .anyMatch(provider -> provider.type() == WalaruTestNgListener.class));
    }

    @Test
    void lifecycle_spi_is_fail_open_without_an_agent() {
        assertDoesNotThrow(() -> {
            WalaruTestLifecycle.started("id", "demo.ExampleTest#works");
            WalaruTestLifecycle.finished("id", "successful", null);
        });
    }
}
