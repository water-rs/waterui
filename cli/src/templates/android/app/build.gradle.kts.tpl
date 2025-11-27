plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// =============================================================================
// WaterUI Rust Build Integration
// =============================================================================
// This task builds the Rust library using `water build android`.
// It runs before the Android build and copies the .so files to jniLibs.

val buildRustLibraries by tasks.registering(Exec::class) {
    description = "Build WaterUI Rust libraries using water CLI"
    group = "build"

    // Find the project root (parent of android directory)
    val projectRoot = rootProject.projectDir.parentFile

    workingDir = projectRoot

    // Determine build type from Gradle's build variant
    val isRelease = gradle.startParameter.taskNames.any {
        it.contains("Release", ignoreCase = true)
    }

    // Build command: water build android [--release] [--targets ...]
    val waterCmd = if (System.getProperty("os.name").lowercase().contains("windows")) {
        listOf("cmd", "/c", "water")
    } else {
        listOf("water")
    }

    val args = mutableListOf<String>()
    args.addAll(waterCmd)
    args.add("build")
    args.add("android")
    args.add("--project")
    args.add(projectRoot.absolutePath)

    if (isRelease) {
        args.add("--release")
    }

    // Optional: Filter targets based on ABI splits or connected device
    // For faster builds during development, you can specify a single target:
    // args.add("--targets")
    // args.add("aarch64-linux-android")

    commandLine = args

    // Environment variables for hot reload (set by water run)
    environment("WATERUI_HOT_RELOAD", System.getenv("WATERUI_HOT_RELOAD") ?: "false")
    System.getenv("WATERUI_HOT_RELOAD_PORT")?.let {
        environment("WATERUI_HOT_RELOAD_PORT", it)
    }

    // Only run if source files changed
    inputs.dir(projectRoot.resolve("src"))
    inputs.file(projectRoot.resolve("Cargo.toml"))
    outputs.dir(projectRoot.resolve("android/app/src/main/jniLibs"))
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
    namespace = "__BUNDLE_IDENTIFIER__"
    compileSdk = 34

    defaultConfig {
        applicationId = "__BUNDLE_IDENTIFIER__"
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    buildFeatures {
        compose = true
    }
    composeOptions {
        kotlinCompilerExtensionVersion = "1.5.14"
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
    compilerOptions {
        freeCompilerArgs.addAll(
            listOf(
                "-P",
                "plugin:androidx.compose.compiler.plugins.kotlin:suppressKotlinVersionCompatibilityCheck=1.9.25"
            )
        )
    }
}

dependencies {
    // Use GitHub dependency in dev mode, local backend otherwise
    if (__USE_DEV_BACKEND__) {
        // JitPack multi-module format: com.github.USER:REPO-SUBMODULE:BRANCH-SNAPSHOT
        implementation("com.github.water-rs:android-backend-runtime:dev-SNAPSHOT")
    } else {
        implementation("dev.waterui.android:runtime")
    }

    val composeBom = platform("androidx.compose:compose-bom:2024.09.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.4")
    implementation("androidx.activity:activity-compose:1.9.1")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("com.google.android.material:material:1.12.0")

    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("junit:junit:4.13.2")
}
