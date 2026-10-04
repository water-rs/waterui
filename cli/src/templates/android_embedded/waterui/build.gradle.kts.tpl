plugins {
    id("com.android.library")
    `maven-publish`
}

android {
    namespace = "{{ ctx.android_package_name() }}.waterui"
    compileSdk = 37

    defaultConfig {
        minSdk = {{ ctx.android_min_api_level() }}
        // Staged JNI-loaded classes need these consumer rules in every variant.
        consumerProguardFiles("proguard-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_{{ ctx.android_jdk_version() }}
        targetCompatibility = JavaVersion.VERSION_{{ ctx.android_jdk_version() }}
    }

    publishing {
        singleVariant("release")
    }
}

group = "{{ ctx.android_package_name() }}"
version = "{{ ctx.crate_version }}"

dependencies {
    // --- begin waterui android classpath dependencies ---
    // --- end waterui android classpath dependencies ---

    // Exported (`api`) so the host's compile classpath sees WaterUiRootView.
    api("{{ ctx.android_runtime_dependency() }}")
}

publishing {
    publications {
        create<MavenPublication>("release") {
            groupId = "{{ ctx.android_package_name() }}"
            artifactId = "{{ ctx.crate_name }}"
            version = "{{ ctx.crate_version }}"

            // `singleVariant("release")` registers the component late, so the
            // publication wires it after evaluation, like the runtime itself.
            afterEvaluate {
                from(components["release"])
            }
        }
    }
}