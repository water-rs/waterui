plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// =============================================================================
// WaterUI Rust Build Integration
// =============================================================================
// This task builds the Rust library for all Android targets using `water build`.
// It runs before the Android build and copies the .so files to jniLibs.
//
// When invoked by `water run`, the Rust build is skipped (water run already
// builds the library). Set WATERUI_SKIP_RUST_BUILD=1 to skip.

// Skip Rust build when invoked by `water run`
val skipRustBuild = System.getenv("WATERUI_SKIP_RUST_BUILD") == "1"

// Android ABI to water CLI architecture mapping
val abiToArch = mapOf(
    "arm64-v8a" to "arm64",
    "armeabi-v7a" to "armv7",
    "x86_64" to "x86-64",
    "x86" to "x86"
)

// Default ABIs to build (can be overridden by setting WATERUI_ANDROID_ABIS env var)
val targetAbis = (System.getenv("WATERUI_ANDROID_ABIS") ?: "arm64-v8a,x86_64")
    .split(",")
    .map { it.trim() }
    .filter { it.isNotEmpty() }

// Find the project root (grandparent of android directory: .water/android -> .water -> project)
val projectRoot = rootProject.projectDir.parentFile.parentFile

// Determine build type from Gradle's build variant
val isRelease = gradle.startParameter.taskNames.any {
    it.contains("Release", ignoreCase = true)
}

// Create build tasks only when not invoked by `water run`
val buildRustTasks = if (skipRustBuild) {
    logger.lifecycle("Skipping Rust build (managed by water run)")
    emptyList()
} else {
    targetAbis.mapNotNull { abi ->
        val arch = abiToArch[abi] ?: run {
            logger.warn("Unknown ABI: $abi, skipping")
            return@mapNotNull null
        }

        tasks.register<Exec>("buildRust_$abi") {
            description = "Build WaterUI Rust library for $abi"
            group = "build"

            workingDir = projectRoot

            val waterCmd = if (System.getProperty("os.name").lowercase().contains("windows")) {
                listOf("cmd", "/c", "water")
            } else {
                listOf("water")
            }

            // Output directory for this ABI's library
            val outputDir = file("src/main/jniLibs/$abi")

            val args = mutableListOf<String>()
            args.addAll(waterCmd)
            args.add("build")
            args.add("--platform")
            args.add("android")
            args.add("--arch")
            args.add(arch)
            args.add("--path")
            args.add(projectRoot.absolutePath)
            args.add("--output-dir")
            args.add(outputDir.absolutePath)

            if (isRelease) {
                args.add("--release")
            }

            commandLine = args

            // Always run - Cargo handles incremental builds internally
            outputs.upToDateWhen { false }
        }
    }
}

// Umbrella task that builds all targets
val buildRustLibraries by tasks.registering {
    description = "Build WaterUI Rust libraries for all Android targets"
    group = "build"
    if (!skipRustBuild) {
        dependsOn(buildRustTasks)
    }
}

// Hook into Android build - run Rust build before compiling
tasks.matching { it.name.startsWith("compile") && it.name.contains("Kotlin") }.configureEach {
    dependsOn(buildRustLibraries)
}

// Also run before native library packaging
tasks.matching { it.name.startsWith("merge") && it.name.contains("JniLibFolders") }.configureEach {
    dependsOn(buildRustLibraries)
}

// =============================================================================

android {
    namespace = "__ANDROID_NAMESPACE__"
    compileSdk = 35

    defaultConfig {
        applicationId = "__BUNDLE_IDENTIFIER__"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        // Filter ABIs to include only the ones we're building for
        // This is set by the CLI via WATERUI_ANDROID_ABIS environment variable
        ndk {
            abiFilters += targetAbis
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    buildFeatures {
        compose = false
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
    kotlinOptions {
        jvmTarget = "21"
    }
    sourceSets {
        getByName("main") {
            jniLibs.srcDir("src/main/jniLibs")
        }
    }
    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

kotlin {
    jvmToolchain(21)
}

dependencies {
    // Use GitHub dependency in remote dev mode, local backend otherwise
    if (__USE_REMOTE_DEV_BACKEND__) {
        // JitPack multi-module format: com.github.USER:REPO-SUBMODULE:BRANCH-SNAPSHOT
        implementation("com.github.water-rs:android-backend:main-SNAPSHOT")
    } else {
        implementation("dev.waterui.android:runtime")
    }

    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.activity:activity-ktx:1.9.3")

    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
    testImplementation("junit:junit:4.13.2")
}
