package io.github.p4suta.walaru.runner

import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.debug.DebugProbes
import org.junit.platform.engine.TestExecutionResult
import org.junit.platform.engine.discovery.DiscoverySelectors
import org.junit.platform.engine.support.descriptor.MethodSource
import org.junit.platform.launcher.TestExecutionListener
import org.junit.platform.launcher.TestIdentifier
import org.junit.platform.launcher.core.LauncherDiscoveryRequestBuilder
import org.junit.platform.launcher.core.LauncherFactory
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicInteger
import org.testng.ITestListener
import org.testng.ITestResult
import org.testng.TestNG
import kotlin.io.path.exists
import kotlin.io.path.extension
import kotlin.io.path.isRegularFile
import kotlin.system.exitProcess

data class RunnerRequest(
    val testRoots: List<Path>,
    val selectors: List<String>,
    val resultFile: Path?,
    val captureCoroutines: Boolean = false,
)

data class StackFrame(
    val className: String,
    val methodName: String,
    val fileName: String?,
    val line: Int,
)

data class FailureDetail(
    val type: String,
    val message: String,
    val frames: List<StackFrame>,
)

data class ExecutedTest(
    val id: String,
    val uniqueId: String,
    val displayName: String,
    val status: String,
    val failure: FailureDetail?,
)

data class CoroutineSnapshot(
    val state: String,
    val context: String,
    val frames: List<StackFrame>,
)

data class RunnerResult(
    val schemaVersion: String = "1",
    val executed: Int,
    val passed: Int,
    val failed: Int,
    val tests: List<ExecutedTest>,
    val coroutines: List<CoroutineSnapshot> = emptyList(),
    val diagnostics: List<String> = emptyList(),
)

/** Lifecycle boundary used by agents and downstream JVM tools without owning the test engine. */
interface RunnerLifecycle {
    fun started(uniqueId: String, publicName: String)

    fun finished(
        uniqueId: String,
        publicName: String,
        status: String,
        failure: Throwable?,
    )
}

object RunnerArguments {
    fun parse(arguments: Array<String>): RunnerRequest {
        val roots = mutableListOf<Path>()
        val selectors = mutableListOf<String>()
        var result: Path? = null
        var coroutines = false
        arguments.forEach { argument ->
            when {
                argument.startsWith("--test-root=") -> roots.add(Path.of(argument.substringAfter('=')))
                argument.startsWith("--select=") -> selectors.add(argument.substringAfter('='))
                argument.startsWith("--result=") -> result = Path.of(argument.substringAfter('='))
                argument == "--coroutines" -> coroutines = true
                else -> throw IllegalArgumentException("unknown runner argument: $argument")
            }
        }
        return RunnerRequest(roots, selectors, result, coroutines)
    }
}

object RunnerEngine {
    fun execute(
        request: RunnerRequest,
        lifecycle: RunnerLifecycle = AgentBridgeLifecycle(),
    ): RunnerResult {
        val diagnostics = mutableListOf<String>()
        val probesInstalled = if (request.captureCoroutines) {
            installDebugProbes(diagnostics)
        } else {
            false
        }
        return try {
            val testNgClasses = TestNgExecution.selectedClasses(request.selectors, request.testRoots)
            val testNgNames = testNgClasses.mapTo(mutableSetOf()) { it.name }
            val junitSelectors = request.selectors.filter { selector ->
                selector.substringBefore('#').removePrefix("testng:") !in testNgNames
            }
            val listener = RecordingListener(lifecycle)
            if (request.selectors.isEmpty() || junitSelectors.isNotEmpty()) {
                val builder = LauncherDiscoveryRequestBuilder.request()
                if (junitSelectors.isEmpty()) {
                    val roots = request.testRoots.filter(Path::exists).toSet()
                    require(roots.isNotEmpty()) { "no existing test roots were provided" }
                    builder.selectors(DiscoverySelectors.selectClasspathRoots(roots))
                } else {
                    builder.selectors(junitSelectors.map(::selector))
                }
                LauncherFactory.create().execute(builder.build(), listener)
            }
            val testNgTests = TestNgExecution.execute(testNgClasses, lifecycle)
            val tests = listener.tests.toList() + testNgTests
            val failed = tests.count { it.status == "failed" }
            RunnerResult(
                executed = tests.size,
                passed = tests.count { it.status == "successful" },
                failed = failed,
                tests = tests,
                coroutines = if (probesInstalled) captureCoroutines(diagnostics) else emptyList(),
                diagnostics = diagnostics,
            )
        } finally {
            if (probesInstalled) uninstallDebugProbes(diagnostics)
        }
    }

    private fun selector(value: String) = when {
        value.startsWith("[") -> DiscoverySelectors.selectUniqueId(value)
        '#' in value -> DiscoverySelectors.selectMethod(value.substringBefore('#'), value.substringAfter('#'))
        else -> DiscoverySelectors.selectClass(value)
    }
}

private object TestNgExecution {
    fun selectedClasses(selectors: List<String>, roots: List<Path>): List<Class<*>> =
        (if (selectors.isEmpty()) discoverNames(roots) else selectors.asSequence()
            .map { it.removePrefix("testng:").substringBefore('#') })
        .distinct()
        .mapNotNull { name ->
            runCatching { Class.forName(name, false, Thread.currentThread().contextClassLoader) }.getOrNull()
        }
        .filter(::isTestNg)
        .sortedBy(Class<*>::getName)
        .toList()

    private fun discoverNames(roots: List<Path>): Sequence<String> = roots.asSequence().flatMap { root ->
        if (!Files.isDirectory(root)) {
            emptySequence()
        } else {
            Files.walk(root).use { files ->
                files.filter { path -> path.isRegularFile() && path.extension == "class" }
                    .map { classFile ->
                        root.relativize(classFile)
                            .toString()
                            .removeSuffix(".class")
                            .replace('/', '.')
                            .replace('\\', '.')
                    }
                    .filter { name -> name != "module-info" && !name.endsWith("package-info") }
                    .toList()
                    .asSequence()
            }
        }
    }

    fun execute(classes: List<Class<*>>, lifecycle: RunnerLifecycle): List<ExecutedTest> {
        if (classes.isEmpty()) return emptyList()
        val listener = ResultListener(lifecycle)
        TestNG().apply {
            setUseDefaultListeners(false)
            setTestClasses(classes.toTypedArray())
            addListener(listener)
        }.run()
        return listener.tests.toList()
    }

    private fun isTestNg(type: Class<*>): Boolean =
        type.declaredAnnotations.any { it.annotationClass.java.name == "org.testng.annotations.Test" } ||
            type.declaredMethods.any { method ->
                method.declaredAnnotations.any { it.annotationClass.java.name == "org.testng.annotations.Test" }
            }

    private class ResultListener(private val lifecycle: RunnerLifecycle) : ITestListener {
        val tests = java.util.Collections.synchronizedList(mutableListOf<ExecutedTest>())

        override fun onTestStart(result: ITestResult) {
            val id = publicName(result)
            lifecycle.started("testng:$id", id)
        }

        override fun onTestSuccess(result: ITestResult) = finished(result, "successful", null)

        override fun onTestFailure(result: ITestResult) = finished(result, "failed", result.throwable)

        override fun onTestSkipped(result: ITestResult) = finished(result, "aborted", result.throwable)

        private fun finished(result: ITestResult, status: String, throwable: Throwable?) {
            val id = publicName(result)
            lifecycle.finished("testng:$id", id, status, throwable)
            tests += ExecutedTest(
                id = id,
                uniqueId = "testng:$id",
                displayName = result.name,
                status = status,
                failure = throwable?.let(::failureDetail),
            )
        }

        private fun publicName(result: ITestResult): String =
            "${result.testClass.realClass.name}#${result.method.methodName}"
    }
}

object RunnerMain {
    @JvmStatic
    fun main(arguments: Array<String>) {
        System.setProperty("walaru.runnerOwnsLifecycle", "true")
        val request = try {
            RunnerArguments.parse(arguments)
        } catch (failure: IllegalArgumentException) {
            System.err.println("walaru-runner: ${failure.message}")
            exitProcess(2)
        }
        val result = try {
            RunnerEngine.execute(request)
        } catch (failure: Throwable) {
            failure.printStackTrace(System.err)
            exitProcess(2)
        }
        request.resultFile?.let { path ->
            path.toAbsolutePath().parent?.let(Files::createDirectories)
            jacksonObjectMapper().writeValue(path.toFile(), result)
        }
        exitProcess(if (result.failed > 0) 1 else 0)
    }
}

private class RecordingListener(private val lifecycle: RunnerLifecycle) : TestExecutionListener {
    val executed = AtomicInteger()
    val tests = mutableListOf<ExecutedTest>()

    override fun executionStarted(testIdentifier: TestIdentifier) {
        if (!testIdentifier.isTest) return
        executed.incrementAndGet()
        lifecycle.started(testIdentifier.uniqueId, publicName(testIdentifier))
    }

    override fun executionFinished(testIdentifier: TestIdentifier, result: TestExecutionResult) {
        if (!testIdentifier.isTest) return
        val id = publicName(testIdentifier)
        val status = result.status.name.lowercase()
        val throwable = result.throwable.orElse(null)
        lifecycle.finished(testIdentifier.uniqueId, id, status, throwable)
        tests += ExecutedTest(
            id = id,
            uniqueId = testIdentifier.uniqueId,
            displayName = testIdentifier.displayName,
            status = status,
            failure = throwable?.let(::failureDetail),
        )
    }

    private fun publicName(identifier: TestIdentifier): String {
        val source = identifier.source.orElse(null)
        return if (source is MethodSource) {
            "${source.className}#${source.methodName}"
        } else {
            identifier.legacyReportingName
        }
    }

}

private fun failureDetail(throwable: Throwable): FailureDetail = FailureDetail(
    type = throwable.javaClass.name,
    message = safeFailureMessage(throwable),
    frames = throwable.stackTrace.take(64).map { frame ->
        StackFrame(frame.className, frame.methodName, frame.fileName, frame.lineNumber)
    },
)

private fun safeFailureMessage(throwable: Throwable): String {
    val trusted = throwable.javaClass.name.startsWith("java.lang.") ||
        throwable.javaClass.name.startsWith("org.opentest4j.") ||
        throwable.javaClass.name.startsWith("org.junit.") ||
        throwable.javaClass.name.startsWith("org.testng.")
    if (!trusted) return "<message unavailable without invoking user code>"
    val value = throwable.message.orEmpty().take(512)
    return value.replace(
        Regex("(?i)(password|secret|token|credential)(\\s*[=:]\\s*)[^\\s,;]+"),
        "$1$2<redacted>",
    )
}

private class AgentBridgeLifecycle : RunnerLifecycle {
    private val bridge = runCatching { Class.forName("io.github.p4suta.walaru.agent.AgentBridge") }.getOrNull()
    private val start = bridge?.getMethod("testStarted", String::class.java, String::class.java)
    private val finish = bridge?.getMethod(
        "testFinished",
        String::class.java,
        String::class.java,
        Throwable::class.java,
    )

    override fun started(uniqueId: String, publicName: String) {
        start?.invoke(null, uniqueId, publicName)
    }

    override fun finished(
        uniqueId: String,
        publicName: String,
        status: String,
        failure: Throwable?,
    ) {
        finish?.invoke(null, uniqueId, status, failure)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
private fun installDebugProbes(diagnostics: MutableList<String>): Boolean = runCatching {
    DebugProbes.install()
    true
}.getOrElse { failure ->
    diagnostics += "DebugProbes unavailable: ${failure.javaClass.name}"
    false
}

@OptIn(ExperimentalCoroutinesApi::class)
private fun uninstallDebugProbes(diagnostics: MutableList<String>) {
    runCatching { DebugProbes.uninstall() }
        .onFailure { failure -> diagnostics += "DebugProbes uninstall failed: ${failure.javaClass.name}" }
}

@OptIn(ExperimentalCoroutinesApi::class)
private fun captureCoroutines(diagnostics: MutableList<String>): List<CoroutineSnapshot> = runCatching {
    DebugProbes.dumpCoroutinesInfo().map { coroutine ->
        CoroutineSnapshot(
            state = coroutine.state.name,
            context = coroutine.context.fold(mutableListOf<String>()) { names, element ->
                names.add(element.javaClass.name)
                names
            }.joinToString(","),
            frames = coroutine.lastObservedStackTrace().take(64).map { frame ->
                StackFrame(frame.className, frame.methodName, frame.fileName, frame.lineNumber)
            },
        )
    }
}.getOrElse { failure ->
    diagnostics += "DebugProbes capture failed: ${failure.javaClass.name}"
    emptyList()
}
