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
    }
}

rootProject.name = "{{ ctx.app_name }}"
include(":app")

// The Hydrolysis Android host is a pinned framework checkout the CLI fetches
// and owns; its host and painter libraries substitute the coordinates the app
// module declares.
includeBuild("{{ ctx.hydrolysis_android_host_project_dir() }}") {
    dependencySubstitution {
        substitute(module("dev.waterui.hydrolysis:host")).using(project(":host"))
        substitute(module("{{ ctx.hydrolysis_android_painter_dependency() }}")).using(project(":{{ ctx.hydrolysis_android_painter_module() }}"))
    }
}
