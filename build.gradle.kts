import org.gradle.api.tasks.testing.logging.TestExceptionFormat
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
        testLogging {
            exceptionFormat = TestExceptionFormat.FULL
            showCauses = true
            showExceptions = true
            showStackTraces = true
        }
    }
    tasks.withType<Javadoc>().configureEach {
        (options as StandardJavadocDocletOptions).addBooleanOption("Xdoclint:all,-missing", true)
        (options as StandardJavadocDocletOptions).addBooleanOption("quiet", true)
    }

    pluginManager.withPlugin("maven-publish") {
        extensions.configure<PublishingExtension> {
            publications.withType<MavenPublication>().configureEach {
                pom {
                    name = project.name
                    url = "https://github.com/P4suta/walaru"
                    licenses {
                        license {
                            name = "Apache License 2.0"
                            url = "https://www.apache.org/licenses/LICENSE-2.0.txt"
                        }
                        license {
                            name = "MIT License"
                            url = "https://opensource.org/license/mit"
                        }
                    }
                    scm {
                        connection = "scm:git:https://github.com/P4suta/walaru.git"
                        developerConnection = "scm:git:ssh://git@github.com/P4suta/walaru.git"
                        url = "https://github.com/P4suta/walaru"
                    }
                    developers {
                        developer {
                            id = "P4suta"
                            name = "P4suta"
                        }
                    }
                }
            }
        }
    }
}
