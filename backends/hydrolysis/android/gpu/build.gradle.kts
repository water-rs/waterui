import org.gradle.api.publish.maven.tasks.AbstractPublishToMaven

plugins {
    id("com.android.library")
    `maven-publish`
}

android {
    namespace = "dev.waterui.hydrolysis.gpu"
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
    api(project(":host"))
}
