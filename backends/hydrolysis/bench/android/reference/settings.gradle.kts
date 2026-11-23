pluginManagement {
    plugins {
        id("com.android.application") version "9.3.0"
        id("com.android.test") version "9.3.0"
        // AGP supplies built-in Kotlin; the Compose compiler matches the
        // Kotlin metadata required by the lazy-list scrollbar.
        id("org.jetbrains.kotlin.plugin.compose") version "2.4.20"
        id("androidx.baselineprofile") version "1.5.0" apply false
    }
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        exclusiveContent {
            forRepository { maven { url = uri("https://jitpack.io") } }
            filter { includeGroup("com.github.nanihadesuka.LazyColumnScrollbar") }
        }
        providers.gradleProperty("benchmarkRepository").orNull?.let { repository ->
            exclusiveContent {
                forRepository { maven { url = uri(repository) } }
                filter { includeGroup("androidx.benchmark") }
            }
        }
    }
}

rootProject.name = "e2e-reference"
include(":app")
// The macrobenchmark and baselineprofile test modules live at
// bench/android/macrobenchmark and bench/android/baselineprofile per the
// section-6 layout; they are modules of this build so their
// `targetProjectPath` can point at the app.
include(":macrobenchmark")
include(":baselineprofile")
project(":macrobenchmark").projectDir = File(rootDir, "../macrobenchmark")
project(":baselineprofile").projectDir = File(rootDir, "../baselineprofile")
