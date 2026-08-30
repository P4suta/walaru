package io.github.p4suta.walaru.gradle

import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property

abstract class WalaruExtension {
    abstract val mode: Property<String>
    abstract val agentJar: Property<String>
    abstract val apiJar: Property<String>
    abstract val eventFile: Property<String>
    abstract val inputFile: Property<String>
    abstract val replayInputFile: Property<String>
    abstract val replayScheduleFile: Property<String>
    abstract val captureFileIo: Property<Boolean>
    abstract val selectedTests: ListProperty<String>
    abstract val modelDirectory: DirectoryProperty
    abstract val reportDirectory: DirectoryProperty
}
