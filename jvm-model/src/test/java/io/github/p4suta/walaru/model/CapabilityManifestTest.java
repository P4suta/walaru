package io.github.p4suta.walaru.model;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

final class CapabilityManifestTest {
    @Test
    void m1ManifestIsExactAboutItsSupportedScope() {
        CapabilityManifest manifest = CapabilityManifest.m1PureJvm();

        assertEquals(Completeness.COMPLETE, manifest.completeness());
        assertTrue(manifest.supported().contains("singleThread"));
        assertTrue(manifest.supported().contains("pureJvm"));
        assertEquals("JNI inputs are not recorded", manifest.unavailable().get("native"));
        assertEquals("child processes are not recorded", manifest.unavailable().get("subprocess"));
    }
}
