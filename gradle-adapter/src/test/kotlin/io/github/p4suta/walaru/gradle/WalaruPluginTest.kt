package io.github.p4suta.walaru.gradle

import org.gradle.testfixtures.ProjectBuilder
import org.junit.jupiter.api.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class WalaruPluginTest {
    @Test
    fun `registers stable model and verification tasks`() {
        val project = ProjectBuilder.builder().build()
        project.pluginManager.apply("java")
        project.pluginManager.apply("io.github.p4suta.walaru")

        assertTrue("walaruModel" in project.tasks.names)
        assertTrue("walaruVerify" in project.tasks.names)
        assertEquals("fast", project.extensions.getByType(WalaruExtension::class.java).mode.get())
        assertEquals(false, project.extensions.getByType(WalaruExtension::class.java).captureFileIo.get())
    }

    @Test
    fun `agent arguments forward explicit bounded file capture without enabling it by default`() {
        val project = ProjectBuilder.builder().build()
        val arguments = project.objects.newInstance(WalaruAgentArguments::class.java)
        arguments.agentJar.set("/opt/walaru/agent.jar")
        arguments.eventFile.set("events.jsonl")
        arguments.mode.set("full")
        arguments.classRoots.set("classes")
        arguments.projectPath.set(":app")
        arguments.captureFileIo.set(true)

        assertTrue("-Dwalaru.captureFileIo=true" in arguments.asArguments())

        arguments.captureFileIo.set(false)
        assertTrue(arguments.asArguments().none { it.startsWith("-Dwalaru.captureFileIo=") })
    }
}
