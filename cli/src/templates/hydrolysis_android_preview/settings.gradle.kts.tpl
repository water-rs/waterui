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
    // The host checkout's catalog declares the Android toolchain the
    // included host build uses; this build applies the same one.
    versionCatalogs {
        create("libs") {
            from(files("{{ ctx.hydrolysis_android_preview_host_project_dir() }}/gradle/libs.versions.toml"))
        }
    }
}

rootProject.name = "waterui-preview-host"
include(":app")

// The Hydrolysis Android host is a pinned checkout the CLI owns; this app's
// `dev.waterui.hydrolysis:preview` dependency resolves to its `:preview`
// module through the composite.
includeBuild("{{ ctx.hydrolysis_android_preview_host_project_dir() }}") {
    name = "hydrolysis-host"
    dependencySubstitution {
        substitute(module("dev.waterui.hydrolysis:preview")).using(project(":preview"))
    }
}
