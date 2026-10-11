import com.android.build.api.dsl.ApplicationExtension
import com.android.build.api.dsl.LibraryExtension
import org.gradle.api.publish.PublishingExtension
import org.gradle.api.publish.maven.MavenPublication
import org.gradle.api.publish.maven.tasks.AbstractPublishToMaven

// `gradle/libs.versions.toml` is the one declaration of the Android toolchain;
// the projects the CLI generates against this checkout import the same file.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.library) apply false
}

val hostCompileSdk: Int = libs.versions.android.compile.sdk.get().toInt()

// Every module compiles against the catalog's compileSdk.
subprojects {
    pluginManager.withPlugin("com.android.library") {
        extensions.configure<LibraryExtension> { compileSdk = hostCompileSdk }
    }
    pluginManager.withPlugin("com.android.application") {
        extensions.configure<ApplicationExtension> { compileSdk = hostCompileSdk }
    }
}

// Every host module that applies `maven-publish` publishes its release
// variant as `dev.waterui.hydrolysis:<module>:<version>`. The CLI stamps each
// embedded build's host version — a content hash of these sources — through
// WATERUI_HYDROLYSIS_HOST_VERSION; a build without it publishes nothing.
subprojects {
    pluginManager.withPlugin("maven-publish") {
        val hostVersion = providers.environmentVariable("WATERUI_HYDROLYSIS_HOST_VERSION")
        group = "dev.waterui.hydrolysis"
        hostVersion.orNull?.let { version = it }

        tasks.withType<AbstractPublishToMaven>().configureEach {
            doFirst {
                check(hostVersion.isPresent) {
                    "WATERUI_HYDROLYSIS_HOST_VERSION must name the host version being published"
                }
            }
        }

        pluginManager.withPlugin("com.android.library") {
            extensions.configure<LibraryExtension> {
                publishing {
                    singleVariant("release")
                }
            }
        }

        extensions.configure<PublishingExtension> {
            publications {
                create<MavenPublication>("release") {
                    artifactId = project.name
                    // `singleVariant("release")` registers the component
                    // late, so the publication wires it after evaluation.
                    afterEvaluate {
                        from(components["release"])
                    }
                }
            }
        }
    }
}
