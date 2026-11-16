import groovy.json.JsonOutput
import java.security.MessageDigest

plugins {
    id("com.android.application") apply false
    id("com.android.test") apply false
    id("org.jetbrains.kotlin.plugin.compose") apply false
    id("androidx.baselineprofile") apply false
}

abstract class RecordBenchmarkToolchain : DefaultTask() {
    @get:Input
    abstract val report: Property<String>

    @get:OutputFile
    abstract val destination: RegularFileProperty

    @TaskAction
    fun writeReport() {
        destination.get().asFile.apply {
            parentFile.mkdirs()
            writeText(report.get())
        }
    }
}

// Keep instrumentation provenance separate from the measured app dependencies.
subprojects {
    plugins.withId("com.android.test") {
        tasks.register<RecordBenchmarkToolchain>("recordBenchmarkToolchain") {
            destination.set(layout.buildDirectory.file("reports/benchmark-toolchain.json"))
            report.set(providers.provider {
                val artifacts = configurations
                    .filter { it.isCanBeResolved && it.name.endsWith("RuntimeClasspath") }
                    .flatMap { configuration ->
                        configuration.resolvedConfiguration.resolvedArtifacts
                            .filter { it.moduleVersion.id.group == "androidx.benchmark" }
                            .map { artifact ->
                                mapOf(
                                    "configuration" to configuration.name,
                                    "coordinate" to artifact.moduleVersion.id.toString(),
                                    "file" to artifact.file.name,
                                    "sha256" to MessageDigest.getInstance("SHA-256")
                                        .digest(artifact.file.readBytes())
                                        .joinToString("") { "%02x".format(it) },
                                )
                            }
                    }
                    .sortedBy { "${it["configuration"]}:${it["coordinate"]}:${it["file"]}" }
                check(artifacts.isNotEmpty()) { "No Benchmark artifacts resolved" }
                val repository = providers.gradleProperty("benchmarkRepository").orNull
                val revision = providers.gradleProperty("benchmarkSourceRevision").orNull
                check(repository == null || !revision.isNullOrBlank()) {
                    "benchmarkRepository requires benchmarkSourceRevision for provenance"
                }
                val report = mapOf(
                    "version" to providers.gradleProperty("benchmarkVersion").get(),
                    "source_revision" to revision,
                    "artifacts" to artifacts,
                )
                JsonOutput.prettyPrint(JsonOutput.toJson(report)) + "\n"
            })
        }
    }
}
