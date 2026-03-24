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
        // Add Maven repository for dev dependencies if using remote dev mode
        if ({{ ctx.use_remote_dev_backend }}) {
            maven {
                url = uri("https://jitpack.io")
            }
        }
    }
}

rootProject.name = "{{ ctx.app_name }}"
include(":app")

// Include the Android backend from the specified path
// For local dev mode: uses waterui repository path directly
// For release mode: uses copied backend in backends/android
if (!{{ ctx.use_remote_dev_backend }}) {
    includeBuild("{{ ctx.android_backend_path() }}") {
        dependencySubstitution {
            substitute(module("dev.waterui.android:runtime")).using(project(":runtime"))
        }
    }
}
