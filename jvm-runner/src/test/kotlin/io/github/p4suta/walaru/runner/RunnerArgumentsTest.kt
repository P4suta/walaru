package io.github.p4suta.walaru.runner

import java.nio.file.Path
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class RunnerArgumentsTest {
    @Test
    fun `parses repeatable roots and selectors without shell interpretation`() {
        val request = RunnerArguments.parse(
            arrayOf(
                "--test-root=/work/build/classes/kotlin/test",
                "--test-root=/work/build/classes/java/test",
                "--select=demo.ExampleTest#works",
                "--result=/tmp/result.json",
            ),
        )

        assertEquals(
            listOf(
                Path.of("/work/build/classes/kotlin/test"),
                Path.of("/work/build/classes/java/test"),
            ),
            request.testRoots,
        )
        assertEquals(listOf("demo.ExampleTest#works"), request.selectors)
        assertEquals(Path.of("/tmp/result.json"), request.resultFile)
    }

    @Test
    fun `rejects unknown flags instead of silently widening execution`() {
        assertFailsWith<IllegalArgumentException> {
            RunnerArguments.parse(arrayOf("--everything=true"))
        }
    }
}
