package io.github.p4suta.walaru.runner

import org.junit.jupiter.api.Test
import java.nio.file.Files
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class RunnerEngineTest {
    @Test
    fun `executes exactly the selected Jupiter method`() {
        val result = RunnerEngine.execute(
            RunnerRequest(
                testRoots = emptyList(),
                selectors = listOf("${PassingFixture::class.java.name}#passes"),
                resultFile = null,
            ),
        )

        assertEquals(1, result.executed)
        assertEquals(1, result.passed)
        assertEquals(0, result.failed)
        assertTrue(result.tests.single().id.endsWith("PassingFixture#passes"))
    }

    @Test
    fun `failure result is structured and stack is bounded`() {
        FailingFixture.failureEnabled = true
        val result = try {
            RunnerEngine.execute(
                RunnerRequest(
                    testRoots = emptyList(),
                    selectors = listOf("${FailingFixture::class.java.name}#fails"),
                    resultFile = null,
                ),
            )
        } finally {
            FailingFixture.failureEnabled = false
        }

        val failure = result.tests.single().failure!!
        assertEquals(1, result.failed)
        assertEquals("java.lang.IllegalStateException", failure.type)
        assertTrue(failure.message.contains("intentional"))
        assertTrue(failure.frames.size <= 64)
        assertTrue(failure.frames.first().className.contains("FailingFixture"))
    }

    @Test
    fun `executes selected TestNG classes through the same structured result`() {
        val result = RunnerEngine.execute(
            RunnerRequest(
                testRoots = emptyList(),
                selectors = listOf(TestNgFixture::class.java.name),
                resultFile = null,
            ),
        )

        assertEquals(1, result.executed)
        assertEquals(1, result.passed)
        assertEquals(0, result.failed)
        assertTrue(result.tests.single().id.endsWith("TestNgFixture#works"))
    }

    @Test
    fun `public lifecycle receives normalized Jupiter and TestNG events`() {
        val events = mutableListOf<String>()
        val lifecycle = object : RunnerLifecycle {
            override fun started(uniqueId: String, publicName: String) {
                events += "start:$publicName"
            }

            override fun finished(uniqueId: String, publicName: String, status: String, failure: Throwable?) {
                events += "finish:$publicName:$status"
            }
        }

        val result = RunnerEngine.execute(
            RunnerRequest(
                testRoots = emptyList(),
                selectors = listOf(
                    "${PassingFixture::class.java.name}#passes",
                    TestNgFixture::class.java.name,
                ),
                resultFile = null,
            ),
            lifecycle,
        )

        assertEquals(2, result.executed)
        assertEquals(
            setOf(
                "start:${PassingFixture::class.java.name}#passes",
                "finish:${PassingFixture::class.java.name}#passes:successful",
                "start:${TestNgFixture::class.java.name}#works",
                "finish:${TestNgFixture::class.java.name}#works:successful",
            ),
            events.toSet(),
        )
    }

    @Test
    fun `discovers TestNG classes from roots when no selectors are supplied`() {
        val root = kotlin.io.path.createTempDirectory("walaru-testng-root")
        val classResource = TestNgFixture::class.java.name.replace('.', '/') + ".class"
        val destination = root.resolve(classResource)
        Files.createDirectories(destination.parent)
        requireNotNull(javaClass.classLoader.getResourceAsStream(classResource)).use { input ->
            Files.copy(input, destination)
        }

        val result = RunnerEngine.execute(
            RunnerRequest(testRoots = listOf(root), selectors = emptyList(), resultFile = null),
        )

        assertTrue(result.tests.any { it.id == "${TestNgFixture::class.java.name}#works" })
    }

    class PassingFixture {
        @Test
        fun passes() = Unit
    }

    class FailingFixture {
        companion object {
            @Volatile
            var failureEnabled: Boolean = false
        }

        @Test
        fun fails() {
            if (failureEnabled) error("intentional failure")
        }
    }

    class TestNgFixture {
        @org.testng.annotations.Test
        fun works() = Unit
    }
}
