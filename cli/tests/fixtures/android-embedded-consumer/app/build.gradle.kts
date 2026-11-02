plugins {
    alias(libs.plugins.android.application)
}

// What the embedded library's render reports: its coordinate, the package
// its generated `WaterUi` lives in, and the SDK levels it requires.
val wateruiLibrary = providers.gradleProperty("wateruiLibrary").get()
val wateruiPackage = providers.gradleProperty("wateruiPackage").get()
val wateruiCompileSdk = providers.gradleProperty("wateruiCompileSdk").get().toInt()
val wateruiMinSdk = providers.gradleProperty("wateruiMinSdk").get().toInt()

// The activity imports the library's generated package, so its source is
// rendered from `src/template` with that package filled in.
val generatedSource = layout.buildDirectory.dir("generated/source/consumer")
val generateConsumerSource = tasks.register<Copy>("generateConsumerSource") {
    from("src/template")
    into(generatedSource)
    inputs.property("wateruiPackage", wateruiPackage)
    expand("wateruiPackage" to wateruiPackage)
}

android {
    namespace = "dev.waterui.embeddedconsumer"
    compileSdk = wateruiCompileSdk

    defaultConfig {
        applicationId = "dev.waterui.embeddedconsumer"
        minSdk = wateruiMinSdk
        targetSdk = wateruiCompileSdk
        versionCode = 1
        versionName = "1.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    sourceSets {
        getByName("main") {
            kotlin.directories.add(generatedSource.get().asFile.path)
        }
    }
}

tasks.named("preBuild") {
    dependsOn(generateConsumerSource)
}

dependencies {
    implementation(wateruiLibrary)
}
