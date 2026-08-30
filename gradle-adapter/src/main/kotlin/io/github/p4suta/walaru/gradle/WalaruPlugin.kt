package io.github.p4suta.walaru.gradle

import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.api.plugins.JavaPluginExtension
import org.gradle.api.tasks.SourceSet
import org.gradle.api.tasks.TaskProvider
import org.gradle.api.tasks.testing.Test
import java.io.File
import java.nio.file.Files

/** Drop-in JVM test intelligence with an embedded API, agent, listener, and local report. */
class WalaruPlugin : Plugin<Project> {
    override fun apply(project: Project) {
        // A CLI init script can apply the packaged plugin before a build's own plugin declaration
        // is resolved from an included build. Those class loaders have distinct plugin identities,
        // so Gradle may invoke apply twice even though both represent Walaru.
        if (
            project.extensions.findByName("walaru") != null &&
            project.tasks.findByName("walaruRuntime") != null
        ) {
            return
        }
        val extension = project.extensions.create("walaru", WalaruExtension::class.java)
        val runtime = project.tasks.register("walaruRuntime", WalaruRuntimeTask::class.java) { task ->
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

        extension.mode.convention(project.providers.systemProperty("walaru.mode").orElse("fast"))
        extension.agentJar.convention(
            project.providers.systemProperty("walaru.agentJar").orElse(
                runtime.flatMap(WalaruRuntimeTask::agentJar).map { it.asFile.absolutePath },
            ),
        )
        extension.apiJar.convention(
            project.providers.systemProperty("walaru.apiJar").orElse(
                runtime.flatMap(WalaruRuntimeTask::apiJar).map { it.asFile.absolutePath },
            ),
        )
        extension.eventFile.convention(
            project.providers.systemProperty("walaru.eventFile").orElse(
                project.layout.buildDirectory.file("walaru/events.jsonl").map { it.asFile.absolutePath },
            ),
        )
        extension.inputFile.convention(project.providers.systemProperty("walaru.inputFile").orElse(""))
        extension.replayInputFile.convention(project.providers.systemProperty("walaru.replayInputFile").orElse(""))
        extension.replayScheduleFile.convention(
            project.providers.systemProperty("walaru.replayScheduleFile").orElse(""),
        )
        extension.captureFileIo.convention(
            project.providers.systemProperty("walaru.captureFileIo").map(String::toBoolean).orElse(false),
        )
        extension.selectedTests.convention(
            project.providers.systemProperty("walaru.tests").map { value ->
                value.split(',').map(String::trim).filter(String::isNotEmpty)
            }.orElse(emptyList()),
        )
        extension.modelDirectory.convention(
            project.layout.dir(project.providers.systemProperty("walaru.modelDirectory").map(::File)).orElse(
                project.rootProject.layout.projectDirectory.dir(".gradle/walaru/model"),
            ),
        )
        extension.reportDirectory.convention(project.layout.buildDirectory.dir("reports/walaru"))

        project.pluginManager.withPlugin("java") {
            configureJavaProject(project, extension, runtime)
        }
    }

    private fun configureJavaProject(
        project: Project,
        extension: WalaruExtension,
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

        val apiFiles = project.files(extension.apiJar)
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
            task.eventFiles.from(extension.eventFile.map(::File))
            task.reportDirectory.set(extension.reportDirectory)
            task.rootDirectory.set(project.rootProject.layout.projectDirectory.asFile.absolutePath)
            task.projectDirectory.set(project.layout.projectDirectory.asFile.absolutePath)
        }

        val externallyManagedEventFile = project.providers.systemProperty("walaru.eventFile").isPresent
        testTask.configure { task ->
            val arguments = project.objects.newInstance(WalaruAgentArguments::class.java)
            arguments.agentJar.set(extension.agentJar)
            arguments.eventFile.set(extension.eventFile)
            arguments.mode.set(extension.mode)
            arguments.classRoots.set(allClassRoots)
            arguments.projectPath.set(project.path)
            arguments.inputFile.set(extension.inputFile)
            arguments.replayInputFile.set(extension.replayInputFile)
            arguments.replayScheduleFile.set(extension.replayScheduleFile)
            arguments.captureFileIo.set(extension.captureFileIo)
            task.jvmArgumentProviders.add(arguments)
            task.dependsOn(runtime)
            task.classpath = task.classpath.plus(project.files(extension.agentJar).builtBy(runtime))
            val requested = extension.selectedTests.get()
            val selected = requested.mapNotNull { selectorForProject(it, project.path) }
            if (requested.isNotEmpty() && selected.isEmpty()) {
                task.enabled = false
            } else {
                selected.forEach(task.filter::includeTestsMatching)
            }
            if (!externallyManagedEventFile) {
                task.doFirst {
                    Files.deleteIfExists(File(extension.eventFile.get()).toPath())
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
            task.outputFile.set(extension.modelDirectory.file("$moduleName.json"))
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
}
