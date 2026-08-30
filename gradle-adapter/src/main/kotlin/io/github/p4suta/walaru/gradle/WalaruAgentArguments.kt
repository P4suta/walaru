package io.github.p4suta.walaru.gradle

import org.gradle.api.provider.Property
import org.gradle.process.CommandLineArgumentProvider
import org.gradle.api.tasks.Input

abstract class WalaruAgentArguments : CommandLineArgumentProvider {
    @get:Input
    abstract val agentJar: Property<String>

    @get:Input
    abstract val eventFile: Property<String>

    @get:Input
    abstract val mode: Property<String>

    @get:Input
    abstract val classRoots: Property<String>

    @get:Input
    abstract val projectPath: Property<String>

    @get:Input
    abstract val inputFile: Property<String>

    @get:Input
    abstract val replayInputFile: Property<String>

    @get:Input
    abstract val replayScheduleFile: Property<String>

    @get:Input
    abstract val captureFileIo: Property<Boolean>

    override fun asArguments(): Iterable<String> {
        val jar = agentJar.get()
        if (jar.isBlank()) return emptyList()
        return buildList {
            add("-javaagent:$jar")
            add("-Dwalaru.eventFile=${eventFile.get()}")
            add("-Dwalaru.mode=${mode.get()}")
            add("-Dwalaru.classRoots=${classRoots.get()}")
            add("-Dwalaru.projectPath=${projectPath.get()}")
            inputFile.orNull?.takeIf(String::isNotBlank)?.let { add("-Dwalaru.inputFile=$it") }
            replayInputFile.orNull?.takeIf(String::isNotBlank)?.let { add("-Dwalaru.replayInputFile=$it") }
            replayScheduleFile.orNull?.takeIf(String::isNotBlank)?.let { add("-Dwalaru.replayScheduleFile=$it") }
            if (captureFileIo.getOrElse(false)) add("-Dwalaru.captureFileIo=true")
        }
    }
}
