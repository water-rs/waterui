pluginManagement {
    repositories {
        google()
        maven { url = uri("https://dl.google.com/dl/android/maven2/") }
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        maven { url = uri("https://dl.google.com/dl/android/maven2/") }
        mavenCentral()
    }
}

rootProject.name = "__APP_NAME__"
include(":app")

// --- New fix: explicitly include Android backend ---
include(":backends:android")
project(":backends:android").projectDir = file("../../backends/android")