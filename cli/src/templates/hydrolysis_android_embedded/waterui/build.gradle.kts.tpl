plugins {
    id("com.android.library")
    `maven-publish`
}

android {
    namespace = "{{ ctx.android_package_name() }}.waterui"
    // The Hydrolysis host modules' compileSdk, which every consumer of
    // this library must reach too.
    compileSdk = {{ ctx.hydrolysis_android_embedded().compile_sdk }}

    defaultConfig {
        minSdk = {{ ctx.hydrolysis_android_embedded().app.min_api_level }}
        // Staged JNI-loaded classes need these consumer rules in every variant.
        consumerProguardFiles("proguard-rules.pro")
        aarMetadata {
            minCompileSdk = {{ ctx.hydrolysis_android_embedded().compile_sdk }}
        }
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

    // Exported (`api`) so the host's compile classpath sees WaterUi and the
    // Hydrolysis embedding types it wraps.
{% for module in ctx.hydrolysis_android_embedded().host_modules -%}
    api("dev.waterui.hydrolysis:{{ module }}:{{ ctx.hydrolysis_android_embedded().host_version }}")
{% endfor -%}
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
