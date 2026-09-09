import groovy.json.JsonSlurper
import org.gradle.api.initialization.resolve.RepositoriesMode

// The Android half of rustls' platform verifier: the Kotlin classes that the
// Rust TLS engine (zenwave over rustls) calls through JNI so that every
// certificate chain is judged by Android's own trust manager. They are not on
// Maven Central; they ship as a Maven repository inside the crate cargo already
// downloaded, so the repository is located from cargo's own metadata, exactly
// as the crate documents.
//
// The crate's version is the Maven artifact's version, so the app pins exactly
// the component that matches the Rust side instead of asking the repository
// for a listing, which an offline build cannot get.
val rustlsPlatformVerifier: Map<String, Any?> = run {
    val metadata = providers.exec {
        workingDir = rootDir.resolve("{{ ctx.project_root_relative_path() }}")
        commandLine(
            "cargo", "metadata",
            "--format-version", "1",
            "--filter-platform", "aarch64-linux-android"
        )
    }.standardOutput.asText.get()

    @Suppress("UNCHECKED_CAST")
    val packages = (JsonSlurper().parseText(metadata) as Map<String, Any?>)["packages"]
        as List<Map<String, Any?>>
    packages.first { it["name"] == "rustls-platform-verifier-android" }
}

fun rustlsPlatformVerifierRepository(): File =
    File(rustlsPlatformVerifier["manifest_path"] as String).parentFile.resolve("maven")

gradle.extra["rustlsPlatformVerifierVersion"] = rustlsPlatformVerifier["version"] as String

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
        maven { url = uri(rustlsPlatformVerifierRepository()) }
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
