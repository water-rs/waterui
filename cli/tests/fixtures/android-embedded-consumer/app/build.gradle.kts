plugins {
    id("com.android.application")
}

// The embedded library's coordinate and the `minSdk` it requires, as the
// library's render reports them.
val wateruiLibrary = providers.gradleProperty("wateruiLibrary").get()
val wateruiMinSdk = providers.gradleProperty("wateruiMinSdk").get().toInt()

android {
    namespace = "dev.waterui.embeddedconsumer"
    compileSdk = 36

    defaultConfig {
        applicationId = "dev.waterui.embeddedconsumer"
        minSdk = wateruiMinSdk
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
}

dependencies {
    implementation(wateruiLibrary)
}
