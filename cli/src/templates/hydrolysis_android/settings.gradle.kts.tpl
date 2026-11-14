import org.gradle.api.initialization.resolve.RepositoriesMode

pluginManagement {
    repositories {
        google()
        maven { url = uri("https://dl.google.com/dl/android/maven2/") }
        mavenCentral()
        gradlePluginPortal()
    }
    plugins {
        // --- begin waterui gradle plugin versions ---
        // --- end waterui gradle plugin versions ---
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        maven { url = uri("https://dl.google.com/dl/android/maven2/") }
        mavenCentral()
    }
    // The host checkout's catalog declares the Android toolchain the
    // included host build uses; this build applies the same one.
    versionCatalogs {
        create("libs") {
            from(files("{{ ctx.hydrolysis_android_host_project_dir() }}/gradle/libs.versions.toml"))
        }
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
{% if ctx.hydrolysis_android_has_system_webview()? %}        substitute(module("dev.waterui.hydrolysis:webview")).using(project(":webview"))
{% endif %}
    }
}
