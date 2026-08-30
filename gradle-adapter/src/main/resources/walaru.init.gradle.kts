initscript {
    val adapterClasspath = System.getProperty("walaru.adapterClasspath")
        ?: System.getProperty("walaru.adapterJar")
    if (!adapterClasspath.isNullOrBlank()) {
        dependencies { classpath(files(adapterClasspath.split(java.io.File.pathSeparator))) }
    }
}

val walaruWorkspaceRoot = System.getProperty("walaru.workspaceRoot")
    ?.let { java.io.File(it).canonicalFile }

gradle.beforeProject {
    if (walaruWorkspaceRoot == null || rootProject.projectDir.canonicalFile == walaruWorkspaceRoot) {
        io.github.p4suta.walaru.gradle.WalaruPlugin.bootstrap(this)
    }
}
