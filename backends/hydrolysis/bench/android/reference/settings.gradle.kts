pluginManagement {
    plugins {
        id("com.android.application") version "9.3.0"
        id("com.android.test") version "9.3.0"
        // AGP 9 ships built-in Kotlin (enabled by default): applying
        // org.jetbrains.kotlin.android separately conflicts with the
        // built-in 'kotlin' extension, so only the Compose compiler plugin
        // is declared here. Built-in Kotlin requires KGP >= 2.2.10 for the
        // compose plugin — 2.2.10 is the smallest compatible bump over the
        // imported 2.2.0 pin.
        id("org.jetbrains.kotlin.plugin.compose") version "2.2.10"
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
