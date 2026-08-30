package io.github.p4suta.walaru.gradle

import io.github.p4suta.walaru.model.GradleProjectModel
import org.gradle.testkit.runner.GradleRunner
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.readText
import kotlin.io.path.writeText
import kotlin.test.assertTrue

class TestNgFunctionalTest {
    @TempDir
    lateinit var projectDirectory: Path

    @Test
    fun `zero config adapter records TestNG lifecycle and values`() {
        fixture("settings.gradle.kts", "rootProject.name = \"testng-fixture\"")
        fixture(
            "build.gradle.kts",
            """
                plugins { java }
                repositories { mavenCentral() }
                dependencies { testImplementation("org.testng:testng:7.11.0") }
                tasks.test { useTestNG() }
            """.trimIndent(),
        )
        fixture(
            "src/test/java/demo/TestNgExample.java",
            """
                package demo;
                public final class TestNgExample {
                    @org.testng.annotations.Test public void works() {
                        org.testng.Assert.assertEquals(2 + 2, 4);
                    }
                }
            """.trimIndent(),
        )
        val initScript = projectDirectory.resolve("walaru.init.gradle.kts")
        Files.copy(checkNotNull(javaClass.getResourceAsStream("/walaru.init.gradle.kts")), initScript)
        val adapterClasspath = listOf(
            WalaruPlugin::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
            GradleProjectModel::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
        ).joinToString(File.pathSeparator)
        val events = projectDirectory.resolve("events.jsonl")

        GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments(
                "walaruVerify",
                "--init-script",
                initScript.toString(),
                "-Dwalaru.adapterClasspath=$adapterClasspath",
                "-Dwalaru.agentJar=${checkNotNull(System.getProperty("walaru.functionalAgent"))}",
                "-Dwalaru.eventFile=$events",
                "-Dwalaru.mode=full",
                "--configuration-cache",
            )
            .build()

        val trace = events.readText()
        assertTrue(trace.contains("\"type\":\"TEST_START\""), trace)
        assertTrue(trace.contains("demo.TestNgExample#works"), trace)
        assertTrue(trace.contains("\"type\":\"LINE\""), trace)
        assertTrue(trace.contains("\"type\":\"TEST_FINISH\""), trace)
    }

    private fun fixture(relative: String, contents: String) {
        val path = projectDirectory.resolve(relative)
        path.parent.createDirectories()
        path.writeText(contents + "\n")
    }
}
