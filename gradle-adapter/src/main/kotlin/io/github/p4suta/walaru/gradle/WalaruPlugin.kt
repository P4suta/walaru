package io.github.p4suta.walaru.gradle

import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.plugins.JavaPluginExtension
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.SourceSet
import org.gradle.api.tasks.TaskProvider
import org.gradle.api.tasks.testing.Test
import java.io.File
import java.nio.file.Files

/** Drop-in JVM test intelligence with an embedded API, agent, listener, and local report. */
class WalaruPlugin : Plugin<Project> {
    override fun apply(project: Project) {
        val internal = internalConfiguration(project)
        val existing = project.extensions.findByName(PUBLIC_EXTENSION)
        if (existing != null) {
            check(existing is WalaruExtension && internal != null) {
                "The '$PUBLIC_EXTENSION' extension is already owned by another plugin"
            }
            return
        }

        if (internal != null) {
            exposePublicExtension(project, internal)
            return
        }
        configureFresh(project, exposePublicExtension = true)
    }

    private fun configureFresh(project: Project, exposePublicExtension: Boolean) {
        if (internalConfiguration(project) != null) return
        val runtime = project.tasks.register(RUNTIME_TASK, WalaruRuntimeTask::class.java) { task ->
            task.group = "verification"
            task.description = "Materializes Walaru's embedded zero-dependency API and Java agent"
            task.runtimeVersion.set(project.version.toString())
            task.externalAgentJar.set(
                project.layout.file(project.providers.systemProperty("walaru.agentJar").map(::File)),
            )
            task.externalApiJar.set(
                project.layout.file(project.providers.systemProperty("walaru.apiJar").map(::File)),
            )
            task.agentJar.set(
                project.layout.buildDirectory.file("walaru/runtime/${project.version}/walaru-agent.jar"),
            )
            task.apiJar.set(
                project.layout.buildDirectory.file("walaru/runtime/${project.version}/walaru-api.jar"),
            )
        }

        val configuration = newInternalConfiguration(project)
        if (exposePublicExtension) exposePublicExtension(project, configuration)

        configuration.mode.convention(project.providers.systemProperty("walaru.mode").orElse("fast"))
        configuration.agentJar.convention(
            project.providers.systemProperty("walaru.agentJar").orElse(
                runtime.flatMap(WalaruRuntimeTask::agentJar).map { it.asFile.absolutePath },
            ),
        )
        configuration.apiJar.convention(
            project.providers.systemProperty("walaru.apiJar").orElse(
                runtime.flatMap(WalaruRuntimeTask::apiJar).map { it.asFile.absolutePath },
            ),
        )
        configuration.eventFile.convention(
            project.providers.systemProperty("walaru.eventFile").orElse(
                project.layout.buildDirectory.file("walaru/events.jsonl").map { it.asFile.absolutePath },
            ),
        )
        configuration.inputFile.convention(project.providers.systemProperty("walaru.inputFile").orElse(""))
        configuration.replayInputFile.convention(
            project.providers.systemProperty("walaru.replayInputFile").orElse(""),
        )
        configuration.replayScheduleFile.convention(
            project.providers.systemProperty("walaru.replayScheduleFile").orElse(""),
        )
        configuration.captureFileIo.convention(
            project.providers.systemProperty("walaru.captureFileIo").map(String::toBoolean).orElse(false),
        )
        configuration.selectedTests.convention(
            project.providers.systemProperty("walaru.tests").map { value ->
                value.split(',').map(String::trim).filter(String::isNotEmpty)
            }.orElse(emptyList()),
        )
        configuration.modelDirectory.convention(
            project.layout.dir(project.providers.systemProperty("walaru.modelDirectory").map(::File)).orElse(
                project.rootProject.layout.projectDirectory.dir(".gradle/walaru/model"),
            ),
        )
        configuration.reportDirectory.convention(project.layout.buildDirectory.dir("reports/walaru"))
        project.extensions.extraProperties.set(INTERNAL_CONFIGURATION, configuration.asBridge())

        project.pluginManager.withPlugin("java") {
            configureJavaProject(project, configuration, runtime)
        }
    }

    private fun exposePublicExtension(project: Project, internal: WalaruConfiguration) {
        project.extensions.create(
            PUBLIC_EXTENSION,
            WalaruExtension::class.java,
            internal.mode,
            internal.agentJar,
            internal.apiJar,
            internal.eventFile,
            internal.inputFile,
            internal.replayInputFile,
            internal.replayScheduleFile,
            internal.captureFileIo,
            internal.selectedTests,
            internal.modelDirectory,
            internal.reportDirectory,
        )
    }

    private fun newInternalConfiguration(project: Project): WalaruConfiguration = WalaruConfiguration(
        mode = project.objects.property(String::class.java),
        agentJar = project.objects.property(String::class.java),
        apiJar = project.objects.property(String::class.java),
        eventFile = project.objects.property(String::class.java),
        inputFile = project.objects.property(String::class.java),
        replayInputFile = project.objects.property(String::class.java),
        replayScheduleFile = project.objects.property(String::class.java),
        captureFileIo = project.objects.property(Boolean::class.java),
        selectedTests = project.objects.listProperty(String::class.java),
        modelDirectory = project.objects.directoryProperty(),
        reportDirectory = project.objects.directoryProperty(),
    )

    private fun WalaruConfiguration.asBridge(): Map<String, Any> = mapOf(
        "mode" to mode,
        "agentJar" to agentJar,
        "apiJar" to apiJar,
        "eventFile" to eventFile,
        "inputFile" to inputFile,
        "replayInputFile" to replayInputFile,
        "replayScheduleFile" to replayScheduleFile,
        "captureFileIo" to captureFileIo,
        "selectedTests" to selectedTests,
        "modelDirectory" to modelDirectory,
        "reportDirectory" to reportDirectory,
    )

    @Suppress("UNCHECKED_CAST")
    private fun internalConfiguration(project: Project): WalaruConfiguration? {
        val extra = project.extensions.extraProperties
        if (!extra.has(INTERNAL_CONFIGURATION)) return null
        val bridge = extra.get(INTERNAL_CONFIGURATION) as? Map<*, *>
            ?: error("Walaru's internal Gradle configuration has an invalid type")
        fun value(name: String): Any = checkNotNull(bridge[name]) {
            "Walaru's internal Gradle configuration is missing '$name'"
        }
        return WalaruConfiguration(
            mode = value("mode") as Property<String>,
            agentJar = value("agentJar") as Property<String>,
            apiJar = value("apiJar") as Property<String>,
            eventFile = value("eventFile") as Property<String>,
            inputFile = value("inputFile") as Property<String>,
            replayInputFile = value("replayInputFile") as Property<String>,
            replayScheduleFile = value("replayScheduleFile") as Property<String>,
            captureFileIo = value("captureFileIo") as Property<Boolean>,
            selectedTests = value("selectedTests") as ListProperty<String>,
            modelDirectory = value("modelDirectory") as DirectoryProperty,
            reportDirectory = value("reportDirectory") as DirectoryProperty,
        )
    }

    private fun configureJavaProject(
        project: Project,
        configuration: WalaruConfiguration,
        runtime: TaskProvider<WalaruRuntimeTask>,
    ) {
        val sourceSets = project.extensions.getByType(JavaPluginExtension::class.java).sourceSets
        val main = sourceSets.getByName(SourceSet.MAIN_SOURCE_SET_NAME)
        val test = sourceSets.getByName(SourceSet.TEST_SOURCE_SET_NAME)
        val testTask = project.tasks.named(
            test.runtimeClasspathConfigurationName.removeSuffix("RuntimeClasspath"),
            Test::class.java,
        )
        val allClassRoots = project.provider {
            (main.output.classesDirs.files + test.output.classesDirs.files)
                .sortedBy { it.absolutePath }
                .joinToString(File.pathSeparator) { it.absolutePath }
        }

        val apiFiles = project.files(configuration.apiJar)
        project.dependencies.add(main.implementationConfigurationName, apiFiles)
        project.tasks.configureEach { task ->
            if (
                task.name == main.compileJavaTaskName ||
                task.name == test.compileJavaTaskName ||
                (task.name.startsWith("compile") && task.name.endsWith("Kotlin"))
            ) {
                task.dependsOn(runtime)
            }
        }

        val report = project.tasks.register("walaruTestReport", WalaruReportTask::class.java) { task ->
            task.group = "verification"
            task.description = "Generates Walaru JSON, Markdown, and self-contained HTML test reports"
            task.eventFiles.from(configuration.eventFile.map(::File))
            task.reportDirectory.set(configuration.reportDirectory)
            task.rootDirectory.set(project.rootProject.layout.projectDirectory.asFile.absolutePath)
            task.projectDirectory.set(project.layout.projectDirectory.asFile.absolutePath)
        }

        val externallyManagedEventFile = project.providers.systemProperty("walaru.eventFile").isPresent
        testTask.configure { task ->
            val arguments = project.objects.newInstance(WalaruAgentArguments::class.java)
            arguments.agentJar.set(configuration.agentJar)
            arguments.eventFile.set(configuration.eventFile)
            arguments.mode.set(configuration.mode)
            arguments.classRoots.set(allClassRoots)
            arguments.projectPath.set(project.path)
            arguments.inputFile.set(configuration.inputFile)
            arguments.replayInputFile.set(configuration.replayInputFile)
            arguments.replayScheduleFile.set(configuration.replayScheduleFile)
            arguments.captureFileIo.set(configuration.captureFileIo)
            task.jvmArgumentProviders.add(arguments)
            task.dependsOn(runtime)
            task.classpath = task.classpath.plus(project.files(configuration.agentJar).builtBy(runtime))
            val requested = configuration.selectedTests.get()
            val selected = requested.mapNotNull { selectorForProject(it, project.path) }
            if (requested.isNotEmpty() && selected.isEmpty()) {
                task.enabled = false
            } else {
                selected.forEach(task.filter::includeTestsMatching)
            }
            if (!externallyManagedEventFile) {
                task.doFirst {
                    Files.deleteIfExists(File(configuration.eventFile.get()).toPath())
                }
                task.finalizedBy(report)
            }
        }
        report.configure { it.mustRunAfter(testTask) }

        val moduleName = if (project.path == ":") "root" else project.path.trim(':').replace(':', '-')
        val model = project.tasks.register("walaruModel", WalaruModelTask::class.java) { task ->
            task.group = "verification"
            task.description = "Exports Walaru's zero-config JVM execution model"
            task.projectPathValue.set(project.path)
            task.testTask.set(testTask.map { it.name })
            task.javaExecutable.set(project.provider {
                testTask.get().javaLauncher.get().executablePath.asFile.absolutePath
            })
            task.productionRoots.from(main.output.classesDirs)
            task.testRoots.from(test.output.classesDirs)
            task.sourceRoots.from(main.allSource.sourceDirectories)
            task.sourceRoots.from(test.allSource.sourceDirectories)
            task.testRuntimeClasspath.from(test.runtimeClasspath)
            task.outputFile.set(configuration.modelDirectory.file("$moduleName.json"))
        }
        val verify = project.tasks.register("walaruVerify") { task ->
            task.group = "verification"
            task.description = "Runs tests with Walaru's safe coverage and dependency intelligence"
            task.dependsOn(model, testTask)
        }
        project.tasks.register("walaruReport") { task ->
            task.group = "verification"
            task.description = "Generates the local Walaru report from the latest test evidence"
            task.dependsOn(report)
        }
        project.tasks.register("walaruExplain") { task ->
            task.group = "verification"
            task.description = "Runs tests and produces an immediately useful local failure explanation"
            task.dependsOn(verify)
        }
    }

    private fun selectorForProject(selector: String, projectPath: String): String? {
        if (!selector.startsWith(':') || "::" !in selector) return selector
        val module = selector.substringBefore("::")
        return selector.substringAfter("::").takeIf { module == projectPath }
    }

    companion object {
        private const val PUBLIC_EXTENSION = "walaru"
        private const val RUNTIME_TASK = "walaruRuntime"
        private const val INTERNAL_CONFIGURATION = "io.github.p4suta.walaru.internalConfiguration.v1"

        /** Configures CLI instrumentation without claiming the build's typed public extension. */
        @JvmStatic
        fun bootstrap(project: Project) {
            WalaruPlugin().configureFresh(project, exposePublicExtension = false)
        }
    }
}

private data class WalaruConfiguration(
    val mode: Property<String>,
    val agentJar: Property<String>,
    val apiJar: Property<String>,
    val eventFile: Property<String>,
    val inputFile: Property<String>,
    val replayInputFile: Property<String>,
    val replayScheduleFile: Property<String>,
    val captureFileIo: Property<Boolean>,
    val selectedTests: ListProperty<String>,
    val modelDirectory: DirectoryProperty,
    val reportDirectory: DirectoryProperty,
)
