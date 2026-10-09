// A minimal existing app that consumes an embedded WaterUI library the way
// `water build` tells a host to: through `mavenLocal()` and the library's
// published coordinate. CI renders `examples/form` as an embedded library
// (`render_embedded_library`), publishes it with its Hydrolysis host
// modules, and compiles this app against the result, passing what the
// render reported as Gradle properties.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        mavenLocal()
        google()
        mavenCentral()
    }
    // The Android toolchain the Hydrolysis host and every project generated
    // against it build with, declared once in the host's catalog.
    versionCatalogs {
        create("libs") {
            from(files("../../../../backends/hydrolysis/android/gradle/libs.versions.toml"))
        }
    }
}

rootProject.name = "embedded-consumer"
include(":app")
