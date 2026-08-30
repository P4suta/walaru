plugins {
    kotlin("jvm")
    `java-gradle-plugin`
}

val functionalAgent = configurations.create("functionalAgent")
val embeddedAgent = configurations.create("embeddedAgent")
val embeddedApi = configurations.create("embeddedApi")

dependencies {
    implementation(project(":jvm-model"))
    testImplementation(gradleTestKit())
    testImplementation(platform(libs.junit.bom))
    testImplementation(kotlin("test"))
    testImplementation(libs.junit.jupiter)
    testImplementation(libs.jackson.databind)
    testRuntimeOnly(libs.junit.launcher)
    functionalAgent(project(path = ":jvm-agent", configuration = "fatJarElements"))
    embeddedAgent(project(path = ":jvm-agent", configuration = "fatJarElements"))
    embeddedApi(project(":jvm-api"))
}

tasks.processResources {
    from(embeddedAgent) {
        into("META-INF/walaru")
        rename { "walaru-agent.jar" }
    }
    from(embeddedApi) {
        into("META-INF/walaru")
        rename { "walaru-api.jar" }
    }
}

val fatJar = tasks.register<Jar>("fatJar") {
    archiveClassifier = "all"
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from(sourceSets.main.get().output)
    dependsOn(configurations.runtimeClasspath)
    from({
        configurations.runtimeClasspath.get().map { dependency ->
            if (dependency.isDirectory) dependency else zipTree(dependency)
        }
    })
}

configurations.create("fatJarElements") {
    isCanBeConsumed = true
    isCanBeResolved = false
    outgoing.artifact(fatJar)
}

tasks.assemble { dependsOn(fatJar) }

tasks.test {
    dependsOn(functionalAgent)
    inputs.files(functionalAgent)
    systemProperty(
        "walaru.functionalAgent",
        project(":jvm-agent").layout.buildDirectory
            .file("libs/jvm-agent-${project.version}-all.jar")
            .get().asFile.absolutePath,
    )
}

gradlePlugin {
    plugins {
        create("walaru") {
            id = "io.github.p4suta.walaru"
            implementationClass = "io.github.p4suta.walaru.gradle.WalaruPlugin"
            displayName = "Walaru adapter"
            description = "Optional acceleration and shared configuration for Walaru"
        }
    }
}
