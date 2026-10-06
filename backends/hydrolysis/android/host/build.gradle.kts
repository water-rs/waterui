plugins {
    id("com.android.library")
}

android {
    namespace = "dev.waterui.hydrolysis"
    compileSdk = 36

    defaultConfig {
        minSdk = 31
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    lint {
        abortOnError = true
        warningsAsErrors = true
        // Dependency freshness turns red the moment a newer release exists
        // upstream; it is not a gate a pull request can pass or fail on its
        // own merits.
        // The toolchain and dependency versions are pinned deliberately.
        disable += "NewerVersionAvailable"
        disable += "AndroidGradlePluginVersion"
        disable += "GradleDependency"
    }
}

dependencies {
    // HydrolysisActivity exposes ComponentActivity in its public API.
    api("androidx.activity:activity:1.11.0")
    implementation("androidx.core:core-ktx:1.17.0")
}
