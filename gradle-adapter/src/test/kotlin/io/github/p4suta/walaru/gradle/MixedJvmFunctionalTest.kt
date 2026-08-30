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

class MixedJvmFunctionalTest {
    @TempDir
    lateinit var projectDirectory: Path

    @Test
    fun `java kotlin jupiter kotest and suspend code emit one structured trace`() {
        fixture("settings.gradle.kts", "rootProject.name = \"mixed-jvm-fixture\"")
        fixture(
            "build.gradle.kts",
            """
                plugins { kotlin("jvm") version "2.4.10" }
                repositories { mavenCentral() }
                dependencies {
                    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
                    testImplementation(platform("org.junit:junit-bom:6.1.3"))
                    testImplementation("org.junit.jupiter:junit-jupiter")
                    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
                    testImplementation("io.kotest:kotest-runner-junit5:6.1.11")
                    testImplementation("io.kotest:kotest-assertions-core:6.1.11")
                }
                tasks.test { useJUnitPlatform() }
            """.trimIndent(),
        )
        fixture(
            "src/main/kotlin/demo/Production.kt",
            """
                package demo
                import kotlinx.coroutines.delay

                class Counter(var value: Int) {
                    fun increment(): Int {
                        value += 1
                        return value
                    }
                }

                suspend fun compute(seed: Int): Int {
                    delay(1)
                    return Counter(seed).increment()
                }
            """.trimIndent(),
        )
        fixture(
            "src/main/java/demo/JavaFacade.java",
            """
                package demo;
                public final class JavaFacade {
                    public static int twice(int value) { return value * 2; }
                }
            """.trimIndent(),
        )
        fixture(
            "src/test/kotlin/demo/JupiterExampleTest.kt",
            """
                package demo
                import kotlinx.coroutines.runBlocking
                import org.junit.jupiter.api.Assertions.assertEquals
                import org.junit.jupiter.api.Test

                class JupiterExampleTest {
                    @Test fun suspendPath() = runBlocking {
                        assertEquals(4, JavaFacade.twice(compute(1)))
                    }
                }
            """.trimIndent(),
        )
        fixture(
            "src/test/kotlin/demo/KotestExample.kt",
            """
                package demo
                import io.kotest.core.spec.style.FunSpec
                import io.kotest.matchers.shouldBe

                class KotestExample : FunSpec({
                    test("suspend kotest path") {
                        compute(2) shouldBe 3
                    }
                })
            """.trimIndent(),
        )
        val initScript = projectDirectory.resolve("walaru.init.gradle.kts")
        Files.copy(checkNotNull(javaClass.getResourceAsStream("/walaru.init.gradle.kts")), initScript)
        val adapterClasspath = listOf(
            WalaruPlugin::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
            GradleProjectModel::class.java.protectionDomain.codeSource.location.toURI().let(Path::of),
        ).joinToString(File.pathSeparator)
        val agent = checkNotNull(System.getProperty("walaru.functionalAgent"))
        val events = projectDirectory.resolve("walaru-events.jsonl")

        GradleRunner.create()
            .withProjectDir(projectDirectory.toFile())
            .withPluginClasspath()
            .withArguments(
                "walaruVerify",
                "--init-script",
                initScript.toString(),
                "-Dwalaru.adapterClasspath=$adapterClasspath",
                "-Dwalaru.agentJar=$agent",
                "-Dwalaru.eventFile=$events",
                "-Dwalaru.mode=full",
                "--configuration-cache",
                "--stacktrace",
            )
            .build()

        val trace = events.readText()
        assertTrue(trace.contains("JupiterExampleTest"), trace)
        assertTrue(trace.contains("KotestExample"), trace)
        assertTrue(trace.contains("\"path\":\"Production.kt\""), trace)
        assertTrue(trace.contains("\"type\":\"LINE\""), trace)
        assertTrue(trace.contains("\"type\":\"CALL\""), trace)
        assertTrue(trace.contains("\"type\":\"WRITE\""), trace)
        assertTrue(trace.contains("\"values\":"), trace)
    }

    private fun fixture(relative: String, contents: String) {
        val path = projectDirectory.resolve(relative)
        path.parent.createDirectories()
        path.writeText(contents + "\n")
    }
}
