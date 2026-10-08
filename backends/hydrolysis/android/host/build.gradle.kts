import org.gradle.api.publish.maven.tasks.AbstractPublishToMaven

plugins {
    id("com.android.library")
    `maven-publish`
}

android {
    namespace = "dev.waterui.hydrolysis"
    compileSdk = 36

    defaultConfig {
        minSdk = 31
        // The @CalledFromNative keep rule the native side's JNI contract
        // needs travels with the library.
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    testOptions {
        // Robolectric's API 36 runtime writes raw FileDescriptor fields
        // through `jdk.internal.access`, which JDK 17+ keeps closed.
        unitTests.all { it.jvmArgs("--add-opens=java.base/jdk.internal.access=ALL-UNNAMED") }
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

    publishing {
        singleVariant("release")
    }
}

// The CLI stamps each build's host version (a content hash of these sources);
// a build without it publishes nothing.
group = "dev.waterui.hydrolysis"
providers.environmentVariable("WATERUI_HYDROLYSIS_HOST_VERSION").orNull?.let { version = it }

tasks.withType<AbstractPublishToMaven>().configureEach {
    doFirst {
        check(providers.environmentVariable("WATERUI_HYDROLYSIS_HOST_VERSION").isPresent) {
            "WATERUI_HYDROLYSIS_HOST_VERSION must name the host version being published"
        }
    }
}

publishing {
    publications {
        create<MavenPublication>("release") {
            artifactId = project.name
            afterEvaluate {
                from(components["release"])
            }
        }
    }
}

dependencies {
    // HydrolysisActivity exposes ComponentActivity in its public API.
    api("androidx.activity:activity:1.11.0")
    implementation("androidx.core:core-ktx:1.17.0")
    // `HintConstants`: the OTP/password hints beyond `View.AUTOFILL_HINT_*`
    // (`AUTOFILL_HINT_SMS_OTP`, `AUTOFILL_HINT_NEW_PASSWORD`).
    implementation("androidx.autofill:autofill:1.3.0")

    // The host's JVM tests run the Kotlin half against a shadowed
    // `NativeBridge` on Robolectric's Android runtime.
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.17")
}