package io.github.p4suta.walaru.model;

import java.util.List;

/** Zero-config model exported by the init-script and optional Gradle plugin. */
public record GradleProjectModel(
        String projectPath,
        String testTask,
        String javaExecutable,
        List<String> productionRoots,
        List<String> testRoots,
        List<String> sourceRoots,
        List<String> testRuntimeClasspath) {
    public GradleProjectModel {
        productionRoots = List.copyOf(productionRoots);
        testRoots = List.copyOf(testRoots);
        sourceRoots = List.copyOf(sourceRoots);
        testRuntimeClasspath = List.copyOf(testRuntimeClasspath);
    }
}
