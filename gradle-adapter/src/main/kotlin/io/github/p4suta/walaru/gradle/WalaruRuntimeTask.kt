package io.github.p4suta.walaru.gradle

import org.gradle.api.DefaultTask
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.Optional
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import java.nio.file.Files
import java.nio.file.StandardCopyOption

/** Materializes the API and agent embedded in the plugin without resolving external repositories. */
abstract class WalaruRuntimeTask : DefaultTask() {
    @get:Input
    abstract val runtimeVersion: Property<String>

    @get:InputFile
    @get:Optional
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val externalAgentJar: RegularFileProperty

    @get:InputFile
    @get:Optional
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val externalApiJar: RegularFileProperty

    @get:OutputFile
    abstract val agentJar: RegularFileProperty

    @get:OutputFile
    abstract val apiJar: RegularFileProperty

    @TaskAction
    fun extract() {
        extract(
            "META-INF/walaru/walaru-agent.jar",
            agentJar.get().asFile.toPath(),
            externalAgentJar.orNull?.asFile?.toPath(),
        )
        extract(
            "META-INF/walaru/walaru-api.jar",
            apiJar.get().asFile.toPath(),
            externalApiJar.orNull?.asFile?.toPath(),
        )
    }

    private fun extract(
        resource: String,
        target: java.nio.file.Path,
        external: java.nio.file.Path?,
    ) {
        Files.createDirectories(target.parent)
        val temporary = Files.createTempFile(target.parent, ".walaru-", ".jar")
        try {
            if (external != null && Files.isRegularFile(external)) {
                Files.copy(external, temporary, StandardCopyOption.REPLACE_EXISTING)
            } else {
                val input = javaClass.classLoader.getResourceAsStream(resource)
                if (input != null) {
                    input.use { Files.copy(it, temporary, StandardCopyOption.REPLACE_EXISTING) }
                } else {
                    val developmentResource = developmentResource(resource)
                    checkNotNull(developmentResource) {
                        "Walaru plugin is missing embedded runtime resource $resource"
                    }
                    Files.copy(developmentResource, temporary, StandardCopyOption.REPLACE_EXISTING)
                }
            }
            try {
                Files.move(
                    temporary,
                    target,
                    StandardCopyOption.ATOMIC_MOVE,
                    StandardCopyOption.REPLACE_EXISTING,
                )
            } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
                Files.move(temporary, target, StandardCopyOption.REPLACE_EXISTING)
            }
        } finally {
            Files.deleteIfExists(temporary)
        }
    }

    private fun developmentResource(resource: String): java.nio.file.Path? {
        val location = runCatching {
            java.nio.file.Path.of(javaClass.protectionDomain.codeSource.location.toURI())
        }.getOrNull() ?: return null
        var candidate: java.nio.file.Path? = location
        repeat(8) {
            val root = candidate ?: return null
            for (path in listOf(root.resolve(resource), root.resolve("resources/main").resolve(resource))) {
                if (Files.isRegularFile(path)) return path
            }
            candidate = root.parent
        }
        return null
    }
}
