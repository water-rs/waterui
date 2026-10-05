// Macrobenchmark test module for the shared section-6 harness.
//
// Carries the instrumented measurements: StartupTimingMetric (COLD and WARM),
// FrameTimingMetric/FrameTimingGfxInfoMetric for the 60-second scripted
// journeys, and PowerMetric.Energy for the ODPM-validated energy pass —
// everything driven by the shared interaction specs so the journeys the
// generator, benchmark and Python replay run are byte-identical.
plugins {
    id("com.android.test")
}

android {
    namespace = "dev.waterui.android.macrobenchmark"
    compileSdk = 37

    defaultConfig {
        minSdk = 31
        targetSdk = 37
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // The app under test is per-fixture flavored; this module's own
        // APK binds the suite variant. run.py selects the fixture's APK
        // through the `fixture` instrumentation arg instead, so one test
        // build covers the whole inventory.
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

dependencies {
    implementation("androidx.test.ext:junit-ktx:1.3.0")
    implementation("androidx.test:runner:1.7.0")
    implementation("androidx.test.uiautomator:uiautomator:2.4.0")
    implementation("androidx.benchmark:benchmark-macro-junit4:1.5.0")
    implementation("org.jetbrains.kotlin:kotlin-stdlib")
}
