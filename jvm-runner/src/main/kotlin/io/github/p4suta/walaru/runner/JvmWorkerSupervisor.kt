package io.github.p4suta.walaru.runner

import java.nio.charset.StandardCharsets
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.security.MessageDigest
import java.time.Duration
import java.util.Comparator
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/** Result of one isolated JVM worker process. */
data class SupervisedJvmResult(
    val exitCode: Int?,
    val timedOut: Boolean,
    val log: Path,
)

/**
 * Public process-tree supervisor shared by Walaru JVM consumers.
 *
 * Arguments are transported with the Java launcher's argument-file syntax so command length and
 * platform shell quoting do not leak into callers. Cancellation, timeout, and host shutdown all
 * terminate descendants before the worker process.
 */
class JvmWorkerSupervisor {
    private val active = AtomicReference<Process?>()

    fun run(
        javaExecutable: Path,
        arguments: List<String>,
        directory: Path,
        timeout: Duration,
        log: Path,
        maxLogBytes: Long = 1024L * 1024L,
    ): SupervisedJvmResult {
        require(!timeout.isNegative && !timeout.isZero) { "timeout must be positive" }
        require(maxLogBytes > 0) { "maxLogBytes must be positive" }
        Files.createDirectories(directory)
        log.toAbsolutePath().parent?.let(Files::createDirectories)
        val digest = MessageDigest.getInstance("SHA-256")
            .digest(arguments.joinToString("\u0000").toByteArray(StandardCharsets.UTF_8))
            .take(6)
            .joinToString("") { byte -> "%02x".format(byte) }
        val argumentFile = directory.resolve("java-$digest.args")
        Files.writeString(
            argumentFile,
            arguments.joinToString("\n", postfix = "\n", transform = ::quoteArgument),
            StandardCharsets.UTF_8,
        )
        val process = ProcessBuilder(javaExecutable.toString(), "@${argumentFile.toAbsolutePath()}")
            .directory(directory.toFile())
            .redirectErrorStream(true)
            .redirectOutput(log.toFile())
            .start()
        check(active.compareAndSet(null, process)) { "this supervisor already owns a worker" }
        val hook = Thread({ terminateTree(process) }, "walaru-jvm-worker-cleanup")
        Runtime.getRuntime().addShutdownHook(hook)
        return try {
            if (process.waitFor(timeout.toMillis().coerceAtLeast(1), TimeUnit.MILLISECONDS)) {
                SupervisedJvmResult(process.exitValue(), false, log)
            } else {
                terminateTree(process)
                SupervisedJvmResult(null, true, log)
            }
        } finally {
            active.compareAndSet(process, null)
            runCatching { Runtime.getRuntime().removeShutdownHook(hook) }
            truncateLog(log, maxLogBytes)
        }
    }

    fun cancel() {
        active.get()?.let(::terminateTree)
    }

    private fun terminateTree(process: Process) {
        process.descendants().sorted(Comparator.reverseOrder()).forEach { child ->
            child.destroy()
            if (child.isAlive) child.destroyForcibly()
        }
        process.destroy()
        if (process.isAlive) process.destroyForcibly()
    }

    private fun quoteArgument(argument: String): String {
        require(argument.none { it == '\u0000' || it == '\n' || it == '\r' }) {
            "JVM worker arguments cannot contain NUL or line breaks"
        }
        return "\"${argument.replace("\\", "\\\\").replace("\"", "\\\"")}\""
    }

    private fun truncateLog(log: Path, maxBytes: Long) {
        if (!Files.exists(log) || Files.size(log) <= maxBytes) return
        val tail = Files.createTempFile(log.toAbsolutePath().parent, ".walaru-log-tail-", ".tmp")
        try {
            FileChannel.open(log, StandardOpenOption.READ).use { input ->
                FileChannel.open(
                    tail,
                    StandardOpenOption.WRITE,
                    StandardOpenOption.TRUNCATE_EXISTING,
                ).use { output ->
                    input.position((input.size() - maxBytes).coerceAtLeast(0))
                    var copied = 0L
                    while (copied < maxBytes) {
                        val count = output.transferFrom(input, copied, maxBytes - copied)
                        if (count <= 0) break
                        copied += count
                    }
                }
            }
            runCatching {
                Files.move(
                    tail,
                    log,
                    StandardCopyOption.REPLACE_EXISTING,
                    StandardCopyOption.ATOMIC_MOVE,
                )
            }.getOrElse {
                Files.move(tail, log, StandardCopyOption.REPLACE_EXISTING)
            }
        } finally {
            Files.deleteIfExists(tail)
        }
    }
}
