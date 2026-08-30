package io.github.p4suta.walaru.gradle

import org.gradle.api.file.DirectoryProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property

/** Typed public view over the classloader-neutral properties used by CLI bootstrap. */
open class WalaruExtension(
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
