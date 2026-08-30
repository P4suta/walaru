package io.github.p4suta.walaru.gradle

import com.fasterxml.jackson.databind.ObjectMapper
import io.github.p4suta.walaru.model.GradleProjectModel
import org.gradle.testkit.runner.GradleRunner
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File
import java.nio.file.Files
import java.nio.file.Path
import kotlin.io.path.createDirectories
import kotlin.io.path.readLines
import kotlin.io.path.writeText
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class MultiModuleFunctionalTest {
    @TempDir
    lateinit var projectDirectory: Path

    @Test
    fun `parallel modules preserve every event and module-qualified test selection`() {
        fixture(
            "settings.gradle.kts",
            """
                rootProject.name = "multi-module-fixture"
                include(":alpha", ":beta")
            """.trimIndent(),
        )
        for (module in listOf("alpha", "beta")) {
            fixture(
                "$module/build.gradle.kts",
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
            fixture(
                "$module/src/test/java/demo/SharedTest.java",
                """
                    package demo;
                    public final class SharedTest {
                        @org.junit.jupiter.api.Test void works() { org.junit.jupiter.api.Assertions.assertTrue(true); }
                    }
                """.trimIndent(),
            )
        }

        val initScript = projectDirectory.resolve("walaru.init.gradle.kts")
        Files.copy(checkNotNull(javaClass.getResourceAsStream("/walaru.init.gradle.kts")), initScript)
        val adapterClasspath = listOf(
            WalaruPlugin::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
            GradleProjectModel::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
        ).joinToString(File.pathSeparator)
        val agent = checkNotNull(System.getProperty("walaru.functionalAgent"))
        val modelDirectory = projectDirectory.resolve("models")
        val allEvents = projectDirectory.resolve("all-events.jsonl")

        runGradle(
            initScript,
            adapterClasspath,
            agent,
            modelDirectory,
            allEvents,
            emptyList(),
        )

        val mapper = ObjectMapper()
        val all = allEvents.readLines().map(mapper::readTree)
        val starts = all.filter { it.path("type").asText() == "TEST_START" }
        assertEquals(
            setOf(":alpha::demo.SharedTest#works", ":beta::demo.SharedTest#works"),
            starts.map { it.path("testName").asText() }.toSet(),
        )
        assertEquals(setOf(":alpha", ":beta"), starts.map { it.path("module").asText() }.toSet())
        assertTrue(modelDirectory.resolve("alpha.json").toFile().isFile)
        assertTrue(modelDirectory.resolve("beta.json").toFile().isFile)

        val selectedEvents = projectDirectory.resolve("selected-events.jsonl")
        runGradle(
            initScript,
            adapterClasspath,
            agent,
            modelDirectory,
            selectedEvents,
            listOf("-Dwalaru.tests=:alpha::demo.SharedTest.works"),
        )
        val selected = selectedEvents.readLines().map(mapper::readTree)
        val selectedStarts = selected.filter { it.path("type").asText() == "TEST_START" }
        assertEquals(listOf(":alpha::demo.SharedTest#works"), selectedStarts.map { it.path("testName").asText() })
    }

    private fun runGradle(
        initScript: Path,
        adapterClasspath: String,
        agent: String,
        modelDirectory: Path,
        events: Path,
        extra: List<String>,
    ) {
        GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments(
                listOf(
                    "walaruVerify",
                    "--parallel",
                    "--rerun-tasks",
                    "--init-script",
                    initScript.toString(),
                    "-Dwalaru.adapterClasspath=$adapterClasspath",
                    "-Dwalaru.agentJar=$agent",
                    "-Dwalaru.eventFile=$events",
                    "-Dwalaru.modelDirectory=$modelDirectory",
                    "-Dwalaru.mode=full",
                    "--configuration-cache",
                    "--stacktrace",
                ) + extra,
            )
            .build()
    }

    private fun fixture(relative: String, contents: String) {
        val path = projectDirectory.resolve(relative)
        path.parent.createDirectories()
        path.writeText(contents + "\n")
    }
}
