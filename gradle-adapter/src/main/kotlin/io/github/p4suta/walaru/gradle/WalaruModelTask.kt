package io.github.p4suta.walaru.gradle

import groovy.json.JsonOutput
import io.github.p4suta.walaru.model.GradleProjectModel
import org.gradle.api.DefaultTask
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Classpath
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction

abstract class WalaruModelTask : DefaultTask() {
    @get:Input
    abstract val projectPathValue: Property<String>

    @get:Input
    abstract val testTask: Property<String>

    @get:Input
    abstract val javaExecutable: Property<String>

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val productionRoots: ConfigurableFileCollection

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val testRoots: ConfigurableFileCollection

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val sourceRoots: ConfigurableFileCollection

    @get:Classpath
    abstract val testRuntimeClasspath: ConfigurableFileCollection

    @get:OutputFile
    abstract val outputFile: RegularFileProperty

    @TaskAction
    fun writeModel() {
        val model = GradleProjectModel(
            projectPathValue.get(),
            testTask.get(),
            javaExecutable.get(),
            productionRoots.files.sortedBy { it.absolutePath }.map { it.absolutePath },
            testRoots.files.sortedBy { it.absolutePath }.map { it.absolutePath },
            sourceRoots.files.sortedBy { it.absolutePath }.map { it.absolutePath },
            testRuntimeClasspath.files.sortedBy { it.absolutePath }.map { it.absolutePath },
        )
        val values = linkedMapOf<String, Any>(
            "projectPath" to model.projectPath(),
            "testTask" to model.testTask(),
            "javaExecutable" to model.javaExecutable(),
            "productionRoots" to model.productionRoots(),
            "testRoots" to model.testRoots(),
            "sourceRoots" to model.sourceRoots(),
            "testRuntimeClasspath" to model.testRuntimeClasspath(),
        )
        val target = outputFile.get().asFile
        target.parentFile.mkdirs()
        target.writeText(JsonOutput.prettyPrint(JsonOutput.toJson(values)) + "\n", Charsets.UTF_8)
    }
}
