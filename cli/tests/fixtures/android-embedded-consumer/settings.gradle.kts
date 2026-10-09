// A minimal existing app that consumes an embedded WaterUI library the way
// `water build` tells a host to: through `mavenLocal()` and the library's
// published coordinate. CI renders `examples/form` as an embedded library
// (`render_embedded_library`), publishes it with its Hydrolysis host
// modules, and compiles this app against the result.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
    plugins {
        id("com.android.application") version "9.3.0"
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        mavenLocal()
        google()
        mavenCentral()
    }
}

rootProject.name = "embedded-consumer"
include(":app")
