pluginManagement {
    plugins {
        id("com.android.application") version "9.3.0"
        id("com.android.library") version "9.3.0"
    }
    repositories {
        google()
        gradlePluginPortal()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "hydrolysis-android"
include(":host", ":gpu", ":hwui", ":test-app")
