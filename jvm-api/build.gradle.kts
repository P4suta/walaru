plugins {
    `java-library`
    `maven-publish`
}

base {
    archivesName = "walaru-api"
}

dependencies {
    testImplementation(platform(libs.junit.bom))
    testImplementation(libs.junit.jupiter)
    testRuntimeOnly(libs.junit.launcher)
}

java {
    withSourcesJar()
    withJavadocJar()
}

publishing {
    publications {
        create<MavenPublication>("library") {
            from(components["java"])
            artifactId = "walaru-api"
            pom {
                name = "Walaru API"
                description = "Zero-dependency capture, checkpoint, and span API for Walaru"
            }
        }
    }
}

tasks.jar {
    manifest {
        attributes(
            "Automatic-Module-Name" to "io.github.p4suta.walaru.api",
            "Implementation-Version" to project.version,
        )
    }
}
