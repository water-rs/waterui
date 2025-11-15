import org.gradle.api.initialization.resolve.RepositoriesMode

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
        // Add Maven repository for dev dependencies if using --dev mode
        if (__USE_DEV_BACKEND__) {
            maven {
                url = uri("https://jitpack.io")
            }
        }
    }
}

rootProject.name = "__APP_NAME__"
include(":app")

// In dev mode, use GitHub dependency; otherwise use local backend
if (!__USE_DEV_BACKEND__) {
    includeBuild("../backends/android") {
        dependencySubstitution {
            substitute(module("dev.waterui.android:runtime")).using(project(":runtime"))
        }
    }
}
