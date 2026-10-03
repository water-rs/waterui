import org.gradle.api.tasks.Exec
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

// The test crate builds to a cdylib per ABI the app packages; a Gradle
// task per ABI drives cargo itself, and the artifacts land in jniLibs
// where AGP picks them up. The device artifact is arm64 alone; extra
// ABIs — an x86_64 emulator build — come in through
// `-Pwaterui.test.abis=arm64-v8a,x86_64`.
val abiToRustTarget = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "x86_64" to "x86_64-linux-android",
    "armeabi-v7a" to "armv7-linux-androideabi",
)
val abis = (findProperty("waterui.test.abis") as String?)
    ?.split(",")
    ?.map(String::trim)
    ?.filter(String::isNotEmpty)
    ?: listOf("arm64-v8a")
val rustProfile = "debug"

val stageTasks = abis.map { abi ->
    val rustTarget = abiToRustTarget[abi]
        ?: error("no Rust target is wired for ABI '$abi'")
    val buildRustLibrary = tasks.register<Exec>("buildRustLibrary${abi.replace("-", "_")}") {
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
    tasks.register<Copy>("stageRustLibrary${abi.replace("-", "_")}") {
        dependsOn(buildRustLibrary)
        from("${rootDir.parentFile.parentFile}/target/$rustTarget/$rustProfile") {
            include("libwaterui_android_test_app.so")
        }
        into("${layout.buildDirectory.get()}/rustJniLibs/$abi")
        rename { "libwaterui_android_test_app.so" }
    }
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
            // arm64 is the shipping ABI; extra ABIs ride the
            // `waterui.test.abis` property for emulator runs.
            abiFilters += abis
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
    dependsOn(stageTasks)
}
