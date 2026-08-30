import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.tasks.KotlinJvmCompile

plugins {
    kotlin("jvm") version "2.4.10" apply false
}

val workspaceVersion = providers.fileContents(layout.projectDirectory.file("Cargo.toml")).asText.map { manifest ->
    val workspacePackage = manifest
        .substringAfter("[workspace.package]", missingDelimiterValue = "")
        .substringBefore("\n[")
    Regex("""(?m)^version\s*=\s*"([^"]+)"\s*$""")
        .find(workspacePackage)
        ?.groupValues
        ?.get(1)
        ?: error("Cargo.toml is missing workspace.package.version")
}

allprojects {
    group = "io.github.p4suta.walaru"
    version = workspaceVersion.get()
}

subprojects {
    tasks.withType<JavaCompile>().configureEach {
        options.release = 21
        options.encoding = "UTF-8"
        options.compilerArgs.add("-Xlint:all")
    }
    tasks.withType<KotlinJvmCompile>().configureEach {
        compilerOptions {
            jvmTarget = JvmTarget.JVM_21
            javaParameters = true
            allWarningsAsErrors = true
        }
    }
    tasks.withType<Test>().configureEach {
        useJUnitPlatform()
        systemProperty("file.encoding", "UTF-8")
    }
}
