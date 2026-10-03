plugins {
    id("com.android.application")
}

android {
    namespace = "dev.waterui.hydrolysis.testapp"
    compileSdk = 36
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "dev.waterui.hydrolysis.testapp"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    sourceSets {
        named("main") {
            jniLibs.srcDir("build/jniLibs")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    lint {
        abortOnError = true
        warningsAsErrors = true
        disable += "NewerVersionAvailable"
        disable += "AndroidGradlePluginVersion"
        disable += "GradleDependency"
        disable += "OldTargetApi"
    }
}

/** Builds the app's Hydrolysis cdylib for every ABI the APK ships. */
val cargoNdk = tasks.register<Exec>("cargoNdk") {
    workingDir = rootProject.file("test-app/rust")
    commandLine(
        "cargo",
        "ndk",
        "--platform",
        "31",
        "--target",
        "arm64-v8a",
        "--target",
        "x86_64",
        "--output-dir",
        rootProject.file("test-app/build/jniLibs").absolutePath,
        "build",
    )
}

tasks.named("preBuild") {
    dependsOn(cargoNdk)
}

dependencies {
    implementation(project(":host"))
    implementation(project(":gpu"))
}
