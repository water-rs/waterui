use anyhow::Result;
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn create_android_project(
    project_dir: &Path,
    app_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
) -> Result<()> {
    let android_dir = project_dir.join("android");
    fs::create_dir_all(&android_dir)?;

    // root build.gradle.kts
    let build_gradle_content = r#"
// Top-level build file where you can add configuration options common to all sub-projects/modules.
plugins {
    id("com.android.application") version "8.2.0" apply false
    id("org.jetbrains.kotlin.android") version "1.9.0" apply false
}
"#;
    let mut build_gradle_file = fs::File::create(android_dir.join("build.gradle.kts"))?;
    build_gradle_file.write_all(build_gradle_content.as_bytes())?;

    // root settings.gradle.kts
    let settings_gradle_content = r#"
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "WaterUI App"
include(":app")
"#;
    let mut settings_gradle_file = fs::File::create(android_dir.join("settings.gradle.kts"))?;
    settings_gradle_file.write_all(settings_gradle_content.as_bytes())?;

    // app module
    let app_dir = android_dir.join("app");
    fs::create_dir_all(&app_dir)?;

    // app/build.gradle.kts
    let app_build_gradle_content = format!(
        r#"
plugins {{
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}}

android {{
    namespace = "{}"
    compileSdk = 34

    defaultConfig {{
        applicationId = "{}"
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }}

    buildTypes {{
        release {{
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }}
    }}
    compileOptions {{
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }}
    kotlinOptions {{
        jvmTarget = "1.8"
    }}
    sourceSets {{
        main {{
            jniLibs.srcDirs = ["src/main/jniLibs"]
        }}
    }}
}}

dependencies {{
    implementation("androidx.core:core-ktx:1.12.0")
    implementation("androidx.appcompat:appcompat:1.6.1")
    implementation("com.google.android.material:material:1.11.0")
    implementation("net.java.dev.jna:jna:5.14.0")
}}
"#,
        bundle_identifier, bundle_identifier
    );
    let mut app_build_gradle_file = fs::File::create(app_dir.join("build.gradle.kts"))?;
    app_build_gradle_file.write_all(app_build_gradle_content.as_bytes())?;

    // app/src/main/AndroidManifest.xml
    let main_dir = app_dir.join("src/main");
    fs::create_dir_all(&main_dir)?;
    let manifest_content = r#"
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">

    <application
        android:allowBackup="true"
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:roundIcon="@mipmap/ic_launcher_round"
        android:supportsRtl="true"
        android:theme="@style/Theme.WaterUIApp">
        <activity
            android:name=".MainActivity"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>

</manifest>
"#
    .to_string();
    let mut manifest_file = fs::File::create(main_dir.join("AndroidManifest.xml"))?;
    manifest_file.write_all(manifest_content.as_bytes())?;

    // MainActivity.kt
    let package_path = bundle_identifier.replace('.', "/");
    let java_dir = main_dir.join(format!("java/{}", package_path));
    fs::create_dir_all(&java_dir)?;
    let main_activity_content = format!(
        r#"
package {}

import androidx.appcompat.app.AppCompatActivity
import android.os.Bundle
// Assuming the backend is in this package
import com.waterui.android.App 

class MainActivity : AppCompatActivity() {{
    override fun onCreate(savedInstanceState: Bundle?) {{
        super.onCreate(savedInstanceState)
        // Load the Rust library
        System.loadLibrary("{}")
        
        // Set the content view to the WaterUI root view
        setContentView(App(this))
    }}
}}
"#,
        bundle_identifier, crate_name
    );
    let mut main_activity_file = fs::File::create(java_dir.join("MainActivity.kt"))?;
    main_activity_file.write_all(main_activity_content.as_bytes())?;

    // res/values
    let res_dir = main_dir.join("res");
    let values_dir = res_dir.join("values");
    fs::create_dir_all(&values_dir)?;

    let strings_content = format!(
        r#"
<resources>
    <string name="app_name">{}</string>
</resources>
"#,
        app_name
    );
    let mut strings_file = fs::File::create(values_dir.join("strings.xml"))?;
    strings_file.write_all(strings_content.as_bytes())?;

    let themes_content = r#"
<resources>
    <!-- Base application theme. -->
    <style name="Theme.WaterUIApp" parent="Theme.Material3.DayNight.NoActionBar">
        <!-- Customize your light theme here. -->
        <!-- <item name="colorPrimary">@color/my_light_primary</item> -->
    </style>
</resources>
"#;
    let mut themes_file = fs::File::create(values_dir.join("themes.xml"))?;
    themes_file.write_all(themes_content.as_bytes())?;

    // build-rust.sh
    let build_rust_sh_content = format!(
        r#"
#!/bin/sh
set -e

# This script builds the Rust library for all Android targets.

# You must have the Android NDK installed and the following environment variables set:
# export ANDROID_NDK_HOME="/path/to/your/android-ndk"
# You also need to install the Rust targets:
# rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

CRATE_NAME={}

# Build for all Android targets
cargo build --target aarch64-linux-android --release
cargo build --target armv7-linux-androideabi --release
cargo build --target i686-linux-android --release
cargo build --target x86_64-linux-android --release

# Copy the libraries to the jniLibs directory
JNI_DIR="android/app/src/main/jniLibs"

mkdir -p "$JNI_DIR/arm64-v8a"
cp "target/aarch64-linux-android/release/lib${{CRATE_NAME}}.so" "$JNI_DIR/arm64-v8a/"

mkdir -p "$JNI_DIR/armeabi-v7a"
cp "target/armv7-linux-androideabi/release/lib${{CRATE_NAME}}.so" "$JNI_DIR/armeabi-v7a/"

mkdir -p "$JNI_DIR/x86"
cp "target/i686-linux-android/release/lib${{CRATE_NAME}}.so" "$JNI_DIR/x86/"

mkdir -p "$JNI_DIR/x86_64"
cp "target/x86_64-linux-android/release/lib${{CRATE_NAME}}.so" "$JNI_DIR/x86_64/"

echo "Rust libraries copied to $JNI_DIR"

"#,
        crate_name
    );
    let mut build_rust_sh_file = fs::File::create(project_dir.join("build-rust.sh"))?;
    build_rust_sh_file.write_all(build_rust_sh_content.as_bytes())?;
    // Make it executable
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(project_dir.join("build-rust.sh"))
        .status()?;

    Ok(())
}
