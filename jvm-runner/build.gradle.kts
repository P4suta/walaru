plugins {
    kotlin("jvm")
    application
}

dependencies {
    implementation(project(":jvm-model"))
    implementation(libs.jackson.databind)
    implementation(libs.jackson.kotlin)
    implementation(platform(libs.junit.bom))
    implementation(libs.junit.launcher)
    implementation(libs.junit.engine)
    implementation(libs.coroutines.debug)
    implementation(libs.testng)

    testImplementation(kotlin("test"))
    testImplementation(libs.junit.jupiter)
    testImplementation(libs.testng)
    testRuntimeOnly(libs.junit.launcher)
}

application {
    mainClass = "io.github.p4suta.walaru.runner.RunnerMain"
}

val fatJar = tasks.register<Jar>("fatJar") {
    archiveClassifier = "all"
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    manifest { attributes("Main-Class" to application.mainClass.get()) }
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
