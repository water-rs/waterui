plugins {
    id("com.android.library")
}

android {
    namespace = "dev.waterui.hydrolysis.webview"
    compileSdk = 36

    defaultConfig {
        minSdk = 31
        // The `AssetResponse.<init>` keep rule the native side's JNI
        // contract needs travels with the library.
        consumerProguardFiles("consumer-rules.pro")
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
    }
}

dependencies {
    api(project(":host"))
    // The `WebViewCompat` surface the bridge is built on: document-start
    // script injection and web-message listeners — the version the Kotlin
    // runtime ships.
    implementation("androidx.webkit:webkit:1.17.1")
}
