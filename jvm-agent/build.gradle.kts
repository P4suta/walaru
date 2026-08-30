plugins {
    `java-library`
}

dependencies {
    implementation(project(":jvm-model"))
    implementation(libs.asm)
    implementation(libs.asm.commons)
    compileOnly(platform(libs.junit.bom))
    compileOnly(libs.junit.launcher)
    compileOnly(libs.testng)

    testImplementation(platform(libs.junit.bom))
    testImplementation(libs.junit.jupiter)
    testImplementation(libs.testng)
    testRuntimeOnly(libs.junit.launcher)
}

tasks.jar {
    manifest {
        attributes(
            "Premain-Class" to "io.github.p4suta.walaru.agent.WalaruAgent",
            "Can-Redefine-Classes" to "false",
            "Can-Retransform-Classes" to "false",
            "Implementation-Version" to project.version,
        )
    }
}

val fatJar = tasks.register<Jar>("fatJar") {
    archiveClassifier = "all"
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    manifest.from(tasks.jar.get().manifest)
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
