import org.gradle.api.tasks.Exec

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

// The test crate builds to a cdylib per ABI the app packages; the Gradle
// task drives cargo itself, and the artifact lands in jniLibs where AGP
// picks it up.
val rustTarget = "aarch64-linux-android"
val abi = "arm64-v8a"
val rustProfile = "debug"

val buildRustLibrary = tasks.register<Exec>("buildRustLibrary") {
    workingDir = rootDir.parentFile.parentFile // the waterui checkout
    commandLine(
        "cargo", "build",
        "--package", "waterui-android-test-app",
        "--target", rustTarget,
        if (rustProfile == "release") "--release" else null,
    ).filterNotNull()
}

val stageRustLibrary = tasks.register<Copy>("stageRustLibrary") {
    dependsOn(buildRustLibrary)
    from("${rootDir.parentFile.parentFile}/target/$rustTarget/$rustProfile") {
        include("libwaterui_android_test_app.so")
    }
    into("${layout.buildDirectory.get()}/rustJniLibs/$abi")
    rename { "libwaterui_android_test_app.so" }
}

android {
    namespace = "dev.waterui.android.test"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "dev.waterui.android.test"
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.compileSdk.get().toInt()
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
    }

    sourceSets {
        named("main") {
            jniLibs.srcDir("${layout.buildDirectory.get()}/rustJniLibs")
        }
    }
}

dependencies {
    implementation(project(":host"))
}

tasks.named("preBuild").configure {
    dependsOn(stageRustLibrary)
}
