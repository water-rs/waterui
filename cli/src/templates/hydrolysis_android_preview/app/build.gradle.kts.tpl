plugins {
    id("com.android.application")
}

android {
    // The namespace must differ from the `:preview` library's — only
    // `applicationId` is the package `run-as` and `am instrument` address.
    namespace = "dev.waterui.hydrolysis.preview.app"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.waterui.hydrolysis.preview"
        minSdk = {{ ctx.hydrolysis_android_preview_min_api_level() }}
        targetSdk = 37
        // The fingerprint of the template and the pinned host's `preview`
        // module: a change in either must reinstall the APK a device already
        // carries, not silently reuse it.
        versionCode = {{ ctx.hydrolysis_android_preview_version_code() }}
        versionName = "preview"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_{{ ctx.android_jdk_version() }}
        targetCompatibility = JavaVersion.VERSION_{{ ctx.android_jdk_version() }}
    }
}

dependencies {
    // The instrumentation and `PreviewBridge` this APK packages, resolved to
    // the pinned host's `:preview` module by `settings.gradle.kts`.
    implementation("dev.waterui.hydrolysis:preview")
}
