// Baseline profile generator module for the shared section-6 harness.
//
// Produces per-variant baseline.prof files from the same interaction
// specifications the benchmarks run, applied by the androidx.baselineprofile
// plugin as generate<Flavor>ReleaseBaselineProfile tasks. The generated
// profile lands in src/<variant>/generated/baselineProfiles and ships inside
// the release APK; benchmarks compile with
// CompilationMode.Partial(BaselineProfileMode.Require) so a missing profile
// fails loudly rather than silently degrading to JIT.
plugins {
    id("com.android.test")
    id("androidx.baselineprofile")
}

android {
    namespace = "dev.waterui.android.baselineprofile"
    compileSdk = 37

    defaultConfig {
        minSdk = 28
        targetSdk = 37
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        missingDimensionStrategy("fixture", "suite")
    }

    targetProjectPath = ":app"

    sourceSets {
        named("main") {
            kotlin.directories.add("../shared/kotlin")
            assets.directories.add("../interactions")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
}

// Connected physical devices only — the Pixel 9 Pro runs generation.
// (API 33+ does not require root.)
baselineProfile {
    useConnectedDevices = true
}

dependencies {
    implementation("androidx.test.ext:junit-ktx:1.3.0")
    implementation("androidx.test:runner:1.7.0")
    implementation("androidx.test.uiautomator:uiautomator:2.4.0")
    implementation("androidx.benchmark:benchmark-macro-junit4:1.5.0")
    implementation("org.jetbrains.kotlin:kotlin-stdlib")
}
