import org.gradle.api.artifacts.dsl.RepositoriesMode

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

includeBuild("../backends/android") {
    dependencySubstitution {
        substitute(module("dev.waterui.android:runtime")).using(project(":runtime"))
    }
}
