package io.github.p4suta.walaru.gradle

import groovy.json.JsonOutput
import groovy.json.JsonSlurper
import org.gradle.api.DefaultTask
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import java.io.ByteArrayOutputStream
import java.nio.charset.StandardCharsets
import java.nio.file.Files

/** Builds a self-contained local report directly from the bounded agent stream. */
abstract class WalaruReportTask : DefaultTask() {
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val eventFiles: ConfigurableFileCollection

    @get:OutputDirectory
    abstract val reportDirectory: DirectoryProperty

    @get:Input
    abstract val rootDirectory: Property<String>

    @get:Input
    abstract val projectDirectory: Property<String>

    @TaskAction
    fun generate() {
        val parser = JsonSlurper()
        val tests = linkedMapOf<String, TestEvidence>()
        var eventCount = 0
        for (source in eventFiles.files.map { it.toPath() }.sorted()) {
            if (!Files.isRegularFile(source) || eventCount >= MAX_EVENTS) continue
            boundedLines(source) { line ->
                if (line.isBlank()) return@boundedLines true
                @Suppress("UNCHECKED_CAST")
                val event = parser.parseText(line) as? Map<String, Any?> ?: return@boundedLines true
                val testName = event["testName"] as? String ?: ""
                if (testName.isBlank()) return@boundedLines true
                val test = tests.getOrPut(testName) { TestEvidence(testName) }
                test.observe(event, sourcePath(event))
                eventCount += 1
                eventCount < MAX_EVENTS
            }
        }

        val output = reportDirectory.get().asFile.toPath()
        Files.createDirectories(output)
        val failures = tests.values.filter { it.status == "failed" }
        val payload = linkedMapOf<String, Any?>(
            "schemaVersion" to "1",
            "summary" to linkedMapOf(
                "tests" to tests.size,
                "passed" to tests.values.count { it.status == "passed" },
                "failed" to failures.size,
                "skipped" to tests.values.count { it.status == "skipped" },
                "events" to eventCount,
            ),
            "tests" to tests.values.map(TestEvidence::asMap),
        )
        Files.writeString(
            output.resolve("report.json"),
            JsonOutput.prettyPrint(JsonOutput.toJson(payload)) + "\n",
            StandardCharsets.UTF_8,
        )
        Files.writeString(output.resolve("report.md"), markdown(tests.values, eventCount), StandardCharsets.UTF_8)
        Files.writeString(output.resolve("index.html"), html(tests.values, eventCount), StandardCharsets.UTF_8)

        val report = output.resolve("index.html").toAbsolutePath().normalize()
        val relative = runCatching {
            java.nio.file.Path.of(rootDirectory.get()).toAbsolutePath().normalize().relativize(report)
        }.getOrDefault(report)
        logger.lifecycle(
            "Walaru: {} tests, {} failed, {} events · {}",
            tests.size,
            failures.size,
            eventCount,
            relative,
        )
    }

    private fun boundedLines(source: java.nio.file.Path, consume: (String) -> Boolean) {
        Files.newInputStream(source).buffered().use { input ->
            val chunk = ByteArray(8 * 1024)
            val line = ByteArrayOutputStream(8 * 1024)
            while (true) {
                val read = input.read(chunk)
                if (read == -1) break
                for (index in 0 until read) {
                    if (chunk[index] == '\n'.code.toByte()) {
                        if (!consume(line.toString(StandardCharsets.UTF_8))) return
                        line.reset()
                    } else {
                        require(line.size() < MAX_EVENT_BYTES) {
                            "Walaru event exceeded $MAX_EVENT_BYTES bytes"
                        }
                        line.write(chunk[index].toInt())
                    }
                }
            }
            if (line.size() > 0) consume(line.toString(StandardCharsets.UTF_8))
        }
    }

    private fun sourcePath(event: Map<String, Any?>): String? {
        val raw = (event["path"] as? String)?.replace('\\', '/') ?: return null
        val safeRaw = raw.take(MAX_SOURCE_PATH)
        val project = java.nio.file.Path.of(projectDirectory.get()).toAbsolutePath().normalize()
        val root = java.nio.file.Path.of(rootDirectory.get()).toAbsolutePath().normalize()
        val rawPath = runCatching { java.nio.file.Path.of(raw) }.getOrNull() ?: return safeRaw
        val direct = rawPath.takeUnless { it.isAbsolute }
            ?.let(project::resolve)
            ?.normalize()
            ?.takeIf { it.startsWith(project) }
        val ownerDirectory = (event["owner"] as? String)
            ?.substringBeforeLast('/', missingDelimiterValue = "")
            .orEmpty()
        val fileName = rawPath.fileName?.toString() ?: return safeRaw
        val packageFile = runCatching {
            java.nio.file.Path.of(ownerDirectory).resolve(fileName).normalize()
        }.getOrNull()?.takeUnless { it.isAbsolute || it.startsWith("..") }
        val candidates = buildList {
            if (direct != null) add(direct)
            if (packageFile != null) {
                for (sourceRoot in listOf("src/main/java", "src/main/kotlin", "src/test/java", "src/test/kotlin")) {
                    val base = project.resolve(sourceRoot).normalize()
                    val candidate = base.resolve(packageFile).normalize()
                    if (candidate.startsWith(base)) add(candidate)
                }
            }
        }
        val resolved = candidates.firstOrNull { Files.isRegularFile(it) } ?: return safeRaw
        return resolved.takeIf { it.startsWith(root) }
            ?.let { root.relativize(it).toString().replace('\\', '/') }
            ?: safeRaw
    }

    private fun markdown(tests: Collection<TestEvidence>, events: Int): String = buildString {
        val failed = tests.filter { it.status == "failed" }
        appendLine("# Walaru test report")
        appendLine()
        appendLine("${tests.size} tests · ${failed.size} failed · $events events")
        if (failed.isEmpty()) {
            appendLine()
            appendLine("All observed tests passed.")
        }
        for (test in failed) {
            val analysis = test.analysis()
            appendLine()
            appendLine("## ${test.name}")
            appendLine()
            appendLine("`${test.failureType}`: ${test.message}")
            appendLine()
            appendLine("### Why this probably failed")
            appendLine()
            appendLine(analysis.summary)
            appendLine()
            appendLine("Likely cause: ${analysis.likelyCause}")
            analysis.focus?.let { appendLine("\nFocus: `${it.first}:${it.second}`") }
            if (test.relevant.isNotEmpty()) {
                appendLine()
                appendLine("Relevant state:")
                for (item in test.relevant.asReversed().take(8)) {
                    appendLine("- ${item.label}: `${item.value}`")
                }
            }
            appendLine()
            appendLine("Next steps:")
            analysis.suggestions.forEach { appendLine("- $it") }
        }
    }

    private fun html(tests: Collection<TestEvidence>, events: Int): String {
        val failed = tests.filter { it.status == "failed" }
        val cards = if (failed.isEmpty()) {
            "<section class=pass><h2>All observed tests passed</h2></section>"
        } else {
            failed.joinToString("\n") { test ->
                val analysis = test.analysis()
                val focus = analysis.focus
                    ?.let { "<p><b>Focus</b> ${escape(it.first)}:${it.second}</p>" }
                    .orEmpty()
                val evidence = test.relevant.asReversed().take(8).joinToString("") { item ->
                    "<li><b>${escape(item.label)}</b><code>${escape(item.value)}</code></li>"
                }
                val suggestions = analysis.suggestions.joinToString("") { "<li>${escape(it)}</li>" }
                """
                <section class=failure>
                  <h2>${escape(test.name)}</h2>
                  <p><b>${escape(test.failureType)}</b>: ${escape(test.message)}</p>
                  <h3>Why this probably failed</h3>
                  <p>${escape(analysis.summary)}</p>
                  <p><b>Likely cause</b> ${escape(analysis.likelyCause)}</p>
                  $focus
                  <h3>Relevant state</h3><ul>$evidence</ul>
                  <h3>Next steps</h3><ol>$suggestions</ol>
                </section>
                """.trimIndent()
            }
        }
        return """
            <!doctype html><html lang=en><meta charset=utf-8>
            <meta name=viewport content="width=device-width,initial-scale=1">
            <title>Walaru test report</title>
            <style>
            :root{color-scheme:light dark;font:15px/1.5 system-ui,sans-serif}body{max-width:960px;margin:3rem auto;padding:0 1.5rem}
            header{display:flex;justify-content:space-between;align-items:baseline;border-bottom:1px solid #8886}.failure,.pass{margin:1.5rem 0;padding:1rem 1.25rem;border-radius:10px;background:#8881}.failure{border-left:5px solid #d33}.pass{border-left:5px solid #2a6}
            code{display:block;overflow-wrap:anywhere;margin:.25rem 0 1rem;padding:.4rem .6rem;background:#8882;border-radius:5px}.hint{opacity:.8}ul{padding-left:1.25rem}
            </style><body><header><h1>Walaru test report</h1><p>${tests.size} tests · ${failed.size} failed · $events events</p></header>
            $cards
            </body></html>
        """.trimIndent() + "\n"
    }

    private data class Relevant(
        val kind: String,
        val label: String,
        val value: String,
        val path: String?,
        val line: Int?,
    )

    private data class Analysis(
        val summary: String,
        val likelyCause: String,
        val focus: Pair<String, Int>?,
        val suggestions: List<String>,
    ) {
        fun asMap(): Map<String, Any?> = linkedMapOf(
            "summary" to summary,
            "likelyCause" to likelyCause,
            "focus" to focus?.let { linkedMapOf("path" to it.first, "line" to it.second) },
            "suggestions" to suggestions,
        )
    }

    private class TestEvidence(val name: String) {
        var status: String = "unknown"
        var failureType: String = ""
        var message: String = ""
        val frames = mutableListOf<String>()
        val relevant = mutableListOf<Relevant>()

        fun observe(event: Map<String, Any?>, sourcePath: String?) {
            val type = event["type"] as? String ?: return
            when (type) {
                "TEST_FINISH" -> {
                    status = when (event["status"] as? String) {
                        "successful" -> "passed"
                        "aborted" -> "skipped"
                        null -> "unknown"
                        else -> event["status"] as String
                    }
                    failureType = event["failureType"] as? String ?: ""
                    message = event["message"] as? String ?: ""
                    (event["frames"] as? Iterable<*>)?.take(16)?.forEach {
                        frames += it?.toString().orEmpty()
                    }
                }
                "CAPTURE", "CHECKPOINT", "NOTE", "SPAN_VALUE", "SPAN_END", "WRITE" -> {
                    @Suppress("UNCHECKED_CAST")
                    val values = event["values"] as? Map<String, Any?> ?: emptyMap()
                    val name = values["name"] as? String ?: ""
                    val label = when (type) {
                        "CAPTURE" -> "Captured `${name.ifBlank { "value" }}`"
                        "CHECKPOINT" -> "Checkpoint `${name.ifBlank { "unnamed" }}`"
                        "NOTE" -> "Note `${name.ifBlank { "unnamed" }}`"
                        "SPAN_VALUE" -> "Span value `${name.ifBlank { "value" }}`"
                        "SPAN_END" -> "Span `${name.ifBlank { "unnamed" }}` ended"
                        else -> "Observed write"
                    }
                    val value = when (type) {
                        "NOTE" -> values["message"]
                        "WRITE" -> values
                        else -> values["value"] ?: values
                    }
                    relevant += Relevant(
                        type,
                        label,
                        JsonOutput.toJson(value).take(MAX_RENDERED_VALUE),
                        sourcePath,
                        (event["line"] as? Number)?.toInt(),
                    )
                    if (relevant.size > MAX_RELEVANT) relevant.removeFirst()
                }
            }
        }

        fun focus(): Pair<String, Int>? = relevant.asReversed().firstNotNullOfOrNull { item ->
            item.path?.let { path -> item.line?.let { line -> path to line } }
        }

        fun analysis(): Analysis {
            val mismatch = assertionMismatch(message)
            val explicit = relevant.asReversed().firstOrNull {
                it.kind != "SPAN_END" && it.value != "\"<redacted>\""
            } ?: relevant.asReversed().firstOrNull()
            val actionableFocus = explicit?.path?.let { path ->
                explicit.line?.let { line -> path to line }
            } ?: focus()
            val likely = explicit?.let {
                "${it.label} with value ${it.value.take(MAX_RENDERED_VALUE)} immediately preceded the failure."
            }
            return when {
                mismatch != null -> Analysis(
                    "Assertion failed: expected ${mismatch.first}, observed ${mismatch.second}.",
                    likely ?: "The assertion compared different values (${mismatch.first} versus ${mismatch.second}).",
                    actionableFocus,
                    listOf(
                        "Inspect the focused line and the latest named captures before the assertion.",
                        "Run `walaru explain` when a full ordered recording is needed.",
                    ),
                )
                failureType.contains("NullPointerException") -> Analysis(
                    "A null value was dereferenced.",
                    likely ?: "A value required by the failing code was null.",
                    actionableFocus,
                    listOf(
                        "Inspect the latest capture feeding the focused line.",
                        "Add `Walaru.capture` at the nullable boundary if the producer is absent.",
                    ),
                )
                failureType.contains("IndexOutOfBounds") -> Analysis(
                    "A collection or array index was outside its valid range.",
                    likely ?: message.take(256),
                    actionableFocus,
                    listOf(
                        "Compare the captured index with the collection length at the focused line.",
                        "Add a checkpoint at the loop or partition boundary when those values are absent.",
                    ),
                )
                else -> Analysis(
                    "${failureType.substringAfterLast('.')}: ${message.take(256)}",
                    likely ?: message.take(256),
                    actionableFocus,
                    listOf(
                        "Inspect the focused source line and recent trace evidence.",
                        "Run `walaru explain` for an ordered full recording of this failure.",
                    ),
                )
            }
        }

        fun asMap(): Map<String, Any?> {
            val analysis = analysis()
            return linkedMapOf(
                "id" to name,
                "status" to status,
                "failureType" to failureType,
                "message" to message,
                "focus" to analysis.focus?.let { linkedMapOf("path" to it.first, "line" to it.second) },
                "analysis" to analysis.asMap(),
                "evidence" to relevant.asReversed().take(8).map {
                    linkedMapOf(
                        "kind" to it.kind.lowercase(),
                        "label" to it.label,
                        "value" to it.value,
                        "path" to it.path,
                        "line" to it.line,
                    )
                },
                "frames" to frames,
            )
        }
    }

    private companion object {
        const val MAX_EVENT_BYTES = 1024 * 1024
        const val MAX_EVENTS = 1_000_000
        const val MAX_RELEVANT = 64
        const val MAX_RENDERED_VALUE = 512
        const val MAX_SOURCE_PATH = 2048

        fun assertionMismatch(message: String): Pair<String, String>? {
            val expectedStart = indexOfIgnoreCase(message, "expected")
                .takeIf { it >= 0 }
                ?.plus("expected".length)
                ?: return null
            val separators = listOf("but was", "but found")
                .mapNotNull { marker ->
                    indexOfIgnoreCase(message, marker, expectedStart)
                        .takeIf { it >= 0 }
                        ?.let { it to marker }
                }
            val (separator, marker) = separators.minByOrNull { it.first } ?: return null
            val expected = assertionValue(message.substring(expectedStart, separator))
            val actual = assertionValue(message.substring(separator + marker.length))
            return if (expected.isBlank() || actual.isBlank()) null else expected to actual
        }

        fun assertionValue(value: String): String = value.trim { character ->
            character.isWhitespace() || character in charArrayOf(':', '=', '<', '[', '(', '.', '>', ']', ')')
        }

        fun indexOfIgnoreCase(value: String, needle: String, start: Int = 0): Int {
            if (needle.isEmpty()) return start.coerceIn(0, value.length)
            for (index in start.coerceAtLeast(0)..value.length - needle.length) {
                if (value.regionMatches(index, needle, 0, needle.length, ignoreCase = true)) return index
            }
            return -1
        }

        fun escape(value: String): String = value
            .replace("&", "&amp;")
            .replace("<", "&lt;")
            .replace(">", "&gt;")
            .replace("\"", "&quot;")
            .replace("'", "&#39;")
    }
}
