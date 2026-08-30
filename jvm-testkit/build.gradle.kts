plugins {
    `java-library`
    `maven-publish`
}

base {
    archivesName = "walaru-testkit"
}

dependencies {
    api(project(":jvm-api"))
    compileOnly(platform(libs.junit.bom))
    compileOnly(libs.junit.launcher)
    compileOnly(libs.testng)

    testImplementation(platform(libs.junit.bom))
    testImplementation(libs.junit.jupiter)
    testImplementation(libs.junit.launcher)
    testImplementation(libs.testng)
}

java {
    withSourcesJar()
    withJavadocJar()
}

publishing {
    publications {
        create<MavenPublication>("library") {
            from(components["java"])
            artifactId = "walaru-testkit"
            pom {
                name = "Walaru Testkit"
                description = "Auto-discovered JUnit Platform and TestNG lifecycle adapters for Walaru"
            }
        }
    }
}

tasks.jar {
    manifest {
        attributes(
            "Automatic-Module-Name" to "io.github.p4suta.walaru.testkit",
            "Implementation-Version" to project.version,
        )
    }
}
