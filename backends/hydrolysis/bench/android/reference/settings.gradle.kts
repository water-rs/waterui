pluginManagement {
    plugins {
        id("com.android.application") version "9.3.0"
        id("org.jetbrains.kotlin.android") version "2.2.0"
        id("org.jetbrains.kotlin.plugin.compose") version "2.2.0"
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
