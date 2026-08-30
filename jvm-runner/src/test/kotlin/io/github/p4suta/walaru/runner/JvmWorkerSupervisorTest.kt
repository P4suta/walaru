package io.github.p4suta.walaru.runner

import java.nio.file.Path
import java.time.Duration
import kotlin.io.path.fileSize
import kotlin.io.path.readText
import kotlin.io.path.writeText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class JvmWorkerSupervisorTest {
    @Test
    fun `runs a JVM through an argument file and captures one bounded log`() {
        val directory = kotlin.io.path.createTempDirectory("walaru-supervisor")
        val log = directory.resolve("worker.log")
        val java = Path.of(System.getProperty("java.home"), "bin", "java")

        val result = JvmWorkerSupervisor().run(
            javaExecutable = java,
            arguments = listOf("-version"),
            directory = directory,
            timeout = Duration.ofSeconds(10),
            log = log,
        )

        assertEquals(0, result.exitCode)
        assertFalse(result.timedOut)
        assertEquals(log, result.log)
        assertTrue(log.readText().contains("version"))
        assertTrue(directory.toFile().listFiles().orEmpty().any { it.name.endsWith(".args") })
    }

    @Test
    fun `times out a stuck JVM and returns only a bounded log tail`() {
        val directory = kotlin.io.path.createTempDirectory("walaru-supervisor-bounds")
        val java = Path.of(System.getProperty("java.home"), "bin", "java")
        val floodSource = directory.resolve("Flood.java")
        floodSource.writeText(
            "public class Flood { public static void main(String[] a) { " +
                "System.out.print(\"A\".repeat(4096)); System.out.print(\"TAIL\"); } }",
        )
        val boundedLog = directory.resolve("bounded.log")

        val flooded = JvmWorkerSupervisor().run(
            javaExecutable = java,
            arguments = listOf(floodSource.toString()),
            directory = directory,
            timeout = Duration.ofSeconds(20),
            log = boundedLog,
            maxLogBytes = 257,
        )

        assertEquals(0, flooded.exitCode)
        assertTrue(boundedLog.fileSize() <= 257)
        assertTrue(boundedLog.readText().endsWith("TAIL"))

        val sleeperSource = directory.resolve("Sleeper.java")
        sleeperSource.writeText(
            "public class Sleeper { public static void main(String[] a) throws Exception { " +
                "Thread.sleep(30000L); } }",
        )
        val timedOut = JvmWorkerSupervisor().run(
            javaExecutable = java,
            arguments = listOf(sleeperSource.toString()),
            directory = directory,
            timeout = Duration.ofMillis(200),
            log = directory.resolve("timeout.log"),
        )

        assertTrue(timedOut.timedOut)
        assertEquals(null, timedOut.exitCode)
    }
}
