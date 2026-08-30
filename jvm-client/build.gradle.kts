plugins {
    `java-library`
    `maven-publish`
}

base {
    archivesName = "walaru-client"
}

dependencies {
    api(libs.jackson.databind)

    testImplementation(platform(libs.junit.bom))
    testImplementation(libs.junit.jupiter)
    testRuntimeOnly(libs.junit.launcher)
}

java {
    withSourcesJar()
    withJavadocJar()
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

tasks.assemble { dependsOn(fatJar) }

publishing {
    publications {
        create<MavenPublication>("library") {
            from(components["java"])
            artifactId = "walaru-client"
            pom {
                name = "Walaru Client"
                description = "Typed, bounded Java and Kotlin client for Walaru"
            }
        }
    }
}

tasks.jar {
    manifest {
        attributes(
            "Automatic-Module-Name" to "io.github.p4suta.walaru.client",
            "Implementation-Version" to project.version,
        )
    }
}
