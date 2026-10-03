import org.gradle.api.tasks.Exec
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

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
        buildList {
            add("cargo")
            add("build")
            add("--locked")
            add("--package")
            add("waterui-android-test-app")
            add("--target")
            add(rustTarget)
            if (rustProfile == "release") add("--release")
        },
    )
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

        ndk {
            // The Rust library builds for arm64 alone for now.
            abiFilters += abi
        }
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        named("main") {
            jniLibs.srcDir("${layout.buildDirectory.get()}/rustJniLibs")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(project(":host"))
}

tasks.named("preBuild").configure {
    dependsOn(stageRustLibrary)
}
