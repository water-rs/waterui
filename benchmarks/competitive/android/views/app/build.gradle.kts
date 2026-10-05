// Android Views contestant — water-rs/waterui#1262.

plugins {
    id("com.android.application")
}

android {
    namespace = "dev.bench.views"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.bench.views"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "1.0"
    }

    splits {
        // Benchmark: per-ABI release APKs (same as every contestant).
        abi {
            isEnable = (project.findProperty("abiSplits") as? String) != "false"
            reset()
            include("arm64-v8a", "x86_64")
            isUniversalApk = false
        }
    }

    signingConfigs {
        named("debug") { storeFile = rootProject.file("debug.keystore") }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("debug")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}


dependencies {
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("androidx.recyclerview:recyclerview:1.4.0")
    implementation("com.google.android.material:material:1.13.0")
}
