package io.github.p4suta.walaru.gradle

import com.fasterxml.jackson.databind.ObjectMapper
import io.github.p4suta.walaru.model.GradleProjectModel
import org.gradle.testkit.runner.GradleRunner
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.nio.file.Files
import java.nio.file.Path
import java.io.File
import kotlin.io.path.createDirectories
import kotlin.io.path.readText
import kotlin.io.path.writeText
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ZeroConfigFunctionalTest {
    @TempDir
    lateinit var projectDirectory: Path

    @Test
    fun `init script discovers test task roots classpath and toolchain without editing target`() {
        fixture("settings.gradle.kts", "rootProject.name = \"zero-config-fixture\"")
        fixture(
            "build.gradle.kts",
            """
                plugins { java }
                repositories { mavenCentral() }
                dependencies {
                    testImplementation(platform("org.junit:junit-bom:6.1.3"))
                    testImplementation("org.junit.jupiter:junit-jupiter")
                    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
                }
                tasks.test { useJUnitPlatform() }
            """.trimIndent(),
        )
        fixture("src/main/java/demo/Example.java", "package demo; public final class Example {}")
        fixture(
            "src/test/java/demo/ExampleTest.java",
            "package demo; public final class ExampleTest { @org.junit.jupiter.api.Test void works() {} }",
        )
        val initScript = projectDirectory.resolve("walaru.init.gradle.kts")
        Files.copy(
            checkNotNull(javaClass.getResourceAsStream("/walaru.init.gradle.kts")),
            initScript,
        )
        val modelDirectory = projectDirectory.resolve("model-output")
        val adapterClasspath = listOf(
            WalaruPlugin::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
            GradleProjectModel::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
        ).joinToString(File.pathSeparator)

        GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments(
                "walaruModel",
                "--init-script",
                initScript.toString(),
                "-Dwalaru.adapterClasspath=$adapterClasspath",
                "-Dwalaru.modelDirectory=$modelDirectory",
                "--configuration-cache",
            )
            .build()

        val modelFile = modelDirectory.resolve("root.json")
        val model = ObjectMapper().readTree(modelFile.toFile())
        assertEquals(":", model.path("projectPath").asText())
        assertEquals("test", model.path("testTask").asText())
        assertTrue(
            model.path("productionRoots").any {
                Path.of(it.asText()).endsWith(Path.of("classes", "java", "main"))
            },
        )
        assertTrue(
            model.path("testRoots").any {
                Path.of(it.asText()).endsWith(Path.of("classes", "java", "test"))
            },
        )
        assertTrue(model.path("testRuntimeClasspath").size() > 0)
        assertTrue(model.path("javaExecutable").asText().contains("java"))
        assertTrue(projectDirectory.resolve("build.gradle.kts").readText().contains("plugins { java }"))
    }

    private fun fixture(relative: String, contents: String) {
        val path = projectDirectory.resolve(relative)
        path.parent.createDirectories()
        path.writeText(contents + "\n")
    }
}
