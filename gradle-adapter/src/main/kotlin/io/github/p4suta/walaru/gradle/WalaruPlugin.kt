package io.github.p4suta.walaru.gradle

import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.api.plugins.JavaPluginExtension
import org.gradle.api.tasks.SourceSet
import org.gradle.api.tasks.testing.Test
import java.io.File

class WalaruPlugin : Plugin<Project> {
    override fun apply(project: Project) {
        val extension = project.extensions.create("walaru", WalaruExtension::class.java)
        extension.mode.convention(project.providers.systemProperty("walaru.mode").orElse("fast"))
        extension.agentJar.convention(project.providers.systemProperty("walaru.agentJar").orElse(""))
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

        project.pluginManager.withPlugin("java") {
            configureJavaProject(project, extension)
        }
    }

    private fun configureJavaProject(project: Project, extension: WalaruExtension) {
        val sourceSets = project.extensions.getByType(JavaPluginExtension::class.java).sourceSets
        val main = sourceSets.getByName(SourceSet.MAIN_SOURCE_SET_NAME)
        val test = sourceSets.getByName(SourceSet.TEST_SOURCE_SET_NAME)
        val testTask = project.tasks.named(test.runtimeClasspathConfigurationName.removeSuffix("RuntimeClasspath"), Test::class.java)
        val allClassRoots = project.provider {
            (main.output.classesDirs.files + test.output.classesDirs.files)
                .sortedBy { it.absolutePath }
                .joinToString(File.pathSeparator) { it.absolutePath }
        }

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
            task.classpath = task.classpath.plus(
                project.files(extension.agentJar.map { jar -> if (jar.isBlank()) emptyList<String>() else listOf(jar) }),
            )
            val requested = extension.selectedTests.get()
            val selected = requested.mapNotNull { selectorForProject(it, project.path) }
            if (requested.isNotEmpty() && selected.isEmpty()) {
                task.enabled = false
            } else {
                selected.forEach(task.filter::includeTestsMatching)
            }
        }

        val moduleName = if (project.path == ":") "root" else project.path.trim(':').replace(':', '-')
        val model = project.tasks.register("walaruModel", WalaruModelTask::class.java) { task ->
            task.group = "verification"
            task.description = "Exports Walaru's zero-config JVM execution model"
            task.projectPathValue.set(project.path)
            task.testTask.set(testTask.map { it.name })
            task.javaExecutable.set(testTask.flatMap { it.javaLauncher }.map { it.executablePath.asFile.absolutePath })
            task.productionRoots.from(main.output.classesDirs)
            task.testRoots.from(test.output.classesDirs)
            task.sourceRoots.from(main.allSource.sourceDirectories)
            task.sourceRoots.from(test.allSource.sourceDirectories)
            task.testRuntimeClasspath.from(test.runtimeClasspath)
            task.outputFile.set(extension.modelDirectory.file("$moduleName.json"))
        }
        project.tasks.register("walaruVerify") { task ->
            task.group = "verification"
            task.description = "Runs the Walaru-configured JUnit Platform test task"
            task.dependsOn(model, testTask)
        }
    }

    private fun selectorForProject(selector: String, projectPath: String): String? {
        if (!selector.startsWith(':') || "::" !in selector) return selector
        val module = selector.substringBefore("::")
        return selector.substringAfter("::").takeIf { module == projectPath }
    }
}
