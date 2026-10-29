plugins {
    id("com.android.application")
}

val painter = providers.gradleProperty("hydrolysis.painter").getOrElse("gpu")
require(painter in setOf("gpu", "hwui")) {
    "hydrolysis.painter must be \"gpu\" or \"hwui\", got \"$painter\""
}
val scene = providers.gradleProperty("hydrolysis.scene").getOrElse("host")
require(scene in setOf("host", "filters")) { "hydrolysis.scene must be host or filters" }

val targetAbis =
    providers.gradleProperty("hydrolysis.abis")
        .getOrElse("arm64-v8a,x86_64")
        .split(',')
require(targetAbis.isNotEmpty() && targetAbis.all { it in setOf("arm64-v8a", "x86_64") }) {
    "hydrolysis.abis must list supported ABIs (arm64-v8a, x86_64), got $targetAbis"
}

android {
    namespace = "dev.waterui.hydrolysis.testapp"
    ndkVersion = "27.2.12479018"

    defaultConfig {
        // The painters install side by side: the HWUI app keeps its own id
        // so both backends stay on a device at once.
        applicationId =
            if (painter == "hwui") "dev.waterui.hydrolysis.testapp.hwui"
            else "dev.waterui.hydrolysis.testapp"
        minSdk = 31
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"

        ndk {
            abiFilters += targetAbis
        }
    }

    sourceSets {
        named("main") {
            kotlin.srcDir("src/$painter/java")
        }
    }

    buildTypes {
        release {
            // Minified builds exercise the host library's consumer keep
            // rules: the @CalledFromNative members must survive R8.
            isMinifyEnabled = true
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

// The Rust cdylib is built per painter and build type: the release APK
// ships the release .so, debug ships debug.
for (buildType in listOf("debug", "release")) {
    val output = rootProject.file("test-app/build/jniLibs/$painter/$scene/$buildType")
    android.sourceSets.named(buildType) {
        jniLibs.srcDir(output)
    }
    val capitalized = buildType.replaceFirstChar { it.uppercaseChar() }
    val cargoTask = tasks.register<Exec>("cargoNdk$capitalized") {
        workingDir = rootProject.file("test-app/rust")
        // wgpu-external-frame's build script compiles its shaders with the
        // NDK's glslc — name the NDK this app declares.
        environment(
            "ANDROID_NDK_HOME",
            androidComponents.sdkComponents.ndkDirectory.get().asFile.absolutePath,
        )
        val args =
            mutableListOf("cargo", "ndk", "--platform", "31")
        targetAbis.forEach { args.addAll(listOf("--target", it)) }
        args.addAll(
            listOf(
                "--output-dir", output.absolutePath,
                "build", "--no-default-features",
                "--features",
                if (scene == "filters") "$painter,filter-probe" else painter,
            ),
        )
        if (buildType == "release") args.add("--release")
        commandLine(args)
    }
    tasks.matching { it.name == "pre${capitalized}Build" }.configureEach {
        dependsOn(cargoTask)
    }
}

dependencies {
    implementation(project(":host"))
    implementation(project(":$painter"))
}
