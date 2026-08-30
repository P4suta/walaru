package io.github.p4suta.walaru.gradle

import io.github.p4suta.walaru.model.GradleProjectModel
import org.gradle.testkit.runner.GradleRunner
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File
import java.nio.file.Files
import java.nio.file.Path
import java.util.jar.JarFile
import kotlin.io.path.createDirectories
import kotlin.io.path.isRegularFile
import kotlin.io.path.readText
import kotlin.io.path.writeText
import kotlin.test.assertTrue

class LibraryFirstFunctionalTest {
    @TempDir
    lateinit var projectDirectory: Path

    @Test
    fun `one plugin adds the API agent lifecycle and useful offline report`() {
        fixture("settings.gradle.kts", "rootProject.name = \"library-first-fixture\"")
        fixture(
            "build.gradle.kts",
            """
                plugins {
                    java
                    id("io.github.p4suta.walaru")
                }
                repositories { mavenCentral() }
                dependencies {
                    testImplementation(platform("org.junit:junit-bom:6.1.3"))
                    testImplementation("org.junit.jupiter:junit-jupiter")
                    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
                }
                tasks.test { useJUnitPlatform() }
            """.trimIndent(),
        )
        fixture(
            "src/main/java/demo/BinarySearch.java",
            """
                package demo;
                import io.github.p4suta.walaru.Walaru;

                public final class BinarySearch {
                    public static int find(int[] values, int target) {
                        try (var span = Walaru.span("binary search").capture("target", target)) {
                            int low = 0;
                            int high = values.length - 1;
                            while (low < high) {
                                int mid = Walaru.capture("mid", (low + high) >>> 1);
                                Walaru.checkpoint("partition", java.util.Map.of("low", low, "high", high));
                                if (values[mid] < target) low = mid + 1;
                                else if (values[mid] > target) high = mid - 1;
                                else return mid;
                            }
                            Walaru.captureRedacted("apiToken", "must-not-leak");
                            return -1;
                        }
                    }
                }
            """.trimIndent(),
        )
        fixture(
            "src/test/java/demo/BinarySearchTest.java",
            """
                package demo;
                import static org.junit.jupiter.api.Assertions.assertEquals;
                import org.junit.jupiter.api.Test;

                final class BinarySearchTest {
                    @Test void findsTheLastElement() {
                        assertEquals(4, BinarySearch.find(new int[] {1, 3, 5, 7, 9}, 9));
                    }
                }
            """.trimIndent(),
        )

        val result = GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments("test", "--configuration-cache", "--stacktrace")
            .buildAndFail()

        val events = projectDirectory.resolve("build/walaru/events.jsonl").readText()
        assertTrue(events.contains("\"type\":\"CAPTURE\""), events)
        assertTrue(events.contains("\"type\":\"CHECKPOINT\""), events)
        assertTrue(events.contains("\"type\":\"SPAN_START\""), events)
        assertTrue(events.contains("\"value\":\"<redacted>\""), events)
        assertTrue(!events.contains("must-not-leak"), events)

        val report = projectDirectory.resolve("build/reports/walaru/report.json")
        val html = projectDirectory.resolve("build/reports/walaru/index.html")
        assertTrue(report.isRegularFile())
        assertTrue(html.isRegularFile())
        assertTrue(report.readText().contains("demo.BinarySearchTest#findsTheLastElement"))
        assertTrue(report.readText().contains("Assertion failed: expected 4, observed -1."))
        assertTrue(report.readText().contains("src/main/java/demo/BinarySearch.java"))
        assertTrue(html.readText().contains("Why this probably failed"))
        assertTrue(html.readText().contains("Relevant state"))
        assertTrue(result.output.contains("Walaru: 1 tests, 1 failed"), result.output)
        val apiJar = projectDirectory.resolve("build/walaru/runtime/unspecified/walaru-api.jar")
        assertTrue(apiJar.isRegularFile())
        assertTrue(projectDirectory.resolve("build/walaru/runtime/unspecified/walaru-agent.jar").isRegularFile())
        JarFile(apiJar.toFile()).use { jar ->
            assertTrue(jar.getEntry("io/github/p4suta/walaru/Walaru.class") != null)
            assertTrue(jar.getEntry("io/github/p4suta/walaru/agent/AgentBridge.class") == null)
        }
    }

    @Test
    fun `the same embedded API is idiomatic from Kotlin`() {
        fixture("settings.gradle.kts", "rootProject.name = \"kotlin-library-fixture\"")
        fixture(
            "build.gradle.kts",
            """
                plugins {
                    kotlin("jvm") version "2.4.10"
                    id("io.github.p4suta.walaru")
                }
                repositories { mavenCentral() }
                dependencies {
                    testImplementation(platform("org.junit:junit-bom:6.1.3"))
                    testImplementation("org.junit.jupiter:junit-jupiter")
                    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
                }
                tasks.test { useJUnitPlatform() }
            """.trimIndent(),
        )
        fixture(
            "src/main/kotlin/demo/Window.kt",
            """
                package demo
                import io.github.p4suta.walaru.Walaru

                fun last(values: List<Int>): Int = Walaru.span("read last").use { span ->
                    span.capture("size", values.size)
                    val index = Walaru.capture("index", values.lastIndex)
                    Walaru.checkpoint("before read", mapOf("index" to index, "size" to values.size))
                    values[index]
                }
            """.trimIndent(),
        )
        fixture(
            "src/test/kotlin/demo/WindowTest.kt",
            """
                package demo
                import org.junit.jupiter.api.Assertions.assertEquals
                import org.junit.jupiter.api.Test

                class WindowTest {
                    @Test fun readsLastValue() = assertEquals(9, last(listOf(1, 3, 9)))
                }
            """.trimIndent(),
        )

        val result = GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments("test", "--configuration-cache", "--stacktrace")
            .build()

        val events = projectDirectory.resolve("build/walaru/events.jsonl").readText()
        assertTrue(events.contains("\"type\":\"CAPTURE\""), events)
        assertTrue(events.contains("\"type\":\"CHECKPOINT\""), events)
        assertTrue(events.contains("\"type\":\"SPAN_END\""), events)
        assertTrue(events.contains("\"path\":\"Window.kt\""), events)
        assertTrue(result.output.contains("Walaru: 1 tests, 0 failed"), result.output)
    }

    @Test
    fun `CLI init script composes with an explicit plugin and leaves included builds alone`() {
        fixture(
            "settings.gradle.kts",
            """
                pluginManagement { includeBuild("support") }
                rootProject.name = "composed-walaru-fixture"
            """.trimIndent(),
        )
        fixture(
            "build.gradle.kts",
            """
                plugins {
                    java
                    id("io.github.p4suta.walaru")
                }
                walaru { mode.set("full") }
                val instrumentedTest = tasks.named<org.gradle.api.tasks.testing.Test>("test")
                check(walaru.mode.get() == "full")
                check("-Dwalaru.mode=full" in instrumentedTest.get().jvmArgumentProviders.flatMap { it.asArguments() })
            """.trimIndent(),
        )
        fixture("support/settings.gradle.kts", "rootProject.name = \"support\"")
        fixture(
            "support/build.gradle.kts",
            """
                plugins { java }
                check(tasks.findByName("walaruRuntime") == null) {
                    "Walaru leaked from the target init script into an included build"
                }
            """.trimIndent(),
        )
        val initScript = projectDirectory.resolve("walaru.init.gradle.kts")
        Files.copy(checkNotNull(javaClass.getResourceAsStream("/walaru.init.gradle.kts")), initScript)
        val adapterClasspath = listOf(
            WalaruPlugin::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
            GradleProjectModel::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
        ).joinToString(File.pathSeparator)

        val result = GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments(
                "tasks",
                "--init-script",
                initScript.toString(),
                "-Dwalaru.adapterClasspath=$adapterClasspath",
                "-Dwalaru.agentJar=${checkNotNull(System.getProperty("walaru.functionalAgent"))}",
                "-Dwalaru.workspaceRoot=${projectDirectory.toFile().canonicalPath}",
                "--configuration-cache",
                "--stacktrace",
            )
            .build()

        assertTrue(result.output.contains("walaruRuntime"), result.output)
        assertTrue(result.output.contains("walaruExplain"), result.output)
    }

    @Test
    fun `local report rejects an oversized event before parsing it`() {
        fixture("settings.gradle.kts", "rootProject.name = \"bounded-report-fixture\"")
        fixture(
            "build.gradle.kts",
            """
                plugins {
                    java
                    id("io.github.p4suta.walaru")
                }
            """.trimIndent(),
        )
        fixture("build/walaru/events.jsonl", "x".repeat(1024 * 1024 + 1))

        val result = GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments("walaruReport", "--stacktrace")
            .buildAndFail()

        assertTrue(result.output.contains("Walaru event exceeded 1048576 bytes"), result.output)
    }

    @Test
    fun `local report parses assertions after unicode text without index drift`() {
        fixture("settings.gradle.kts", "rootProject.name = \"unicode-report-fixture\"")
        fixture(
            "build.gradle.kts",
            """
                plugins {
                    java
                    id("io.github.p4suta.walaru")
                }
            """.trimIndent(),
        )
        fixture(
            "build/walaru/events.jsonl",
            """{"type":"TEST_FINISH","testName":"demo.UnicodeTest#works","status":"failed","failureType":"org.opentest4j.AssertionFailedError","message":"İstanbul EXPECTED: <4> but found: <5>"}""",
        )

        GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments("walaruReport", "--stacktrace")
            .build()

        assertTrue(
            projectDirectory.resolve("build/reports/walaru/report.json").readText()
                .contains("Assertion failed: expected 4, observed 5."),
        )
    }

    private fun fixture(relative: String, contents: String) {
        val path = projectDirectory.resolve(relative)
        path.parent.createDirectories()
        path.writeText(contents + "\n")
    }
}
