initscript {
    val adapterClasspath = System.getProperty("walaru.adapterClasspath")
        ?: System.getProperty("walaru.adapterJar")
    if (!adapterClasspath.isNullOrBlank()) {
        dependencies { classpath(files(adapterClasspath.split(java.io.File.pathSeparator))) }
    }
}

gradle.beforeProject {
    pluginManager.apply(io.github.p4suta.walaru.gradle.WalaruPlugin::class.java)
}
