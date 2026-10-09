plugins {
    id("com.android.library")
}

android {
    namespace = "dev.waterui.hydrolysis.hwui"
    compileSdk = 36

    defaultConfig {
        // The framework floor (`android-min-api-level` in the workspace
        // metadata). Mesh draws (34) and runtime shaders (33) check the
        // device level on the Rust side before they are encoded.
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
}

dependencies {
    api(project(":host"))
    implementation("androidx.annotation:annotation:1.8.1")
    implementation("androidx.core:core:1.17.0")
    testImplementation("junit:junit:4.13.2")
}

// The Rust half of the contract test encodes its scenes at test time, and
// `hwui::kotlin` renders the Kotlin wire constants (`Protocol.kt`) from the
// Rust definitions. Neither output is committed: `ContractTest` decodes the
// scenes, and the generated source is compiled into the module.
abstract class ContractScenes : Exec() {
    @get:OutputDirectory
    abstract val scenes: DirectoryProperty

    @get:OutputDirectory
    abstract val wire: DirectoryProperty
}

val repositoryRoot = rootDir.resolve("../../..")

val contractScenes =
    tasks.register<ContractScenes>("contractScenes") {
        description = "Encodes the HWUI contract scenes and generates the Kotlin wire from the Rust definitions."
        scenes.set(layout.buildDirectory.dir("contract"))
        wire.set(layout.buildDirectory.dir("generated/source/hwuiWire"))
        workingDir = repositoryRoot
        commandLine(
            "cargo", "test", "--locked", "-p", "hydrolysis", "--lib", "--",
            "hwui::contract", "hwui::kotlin",
        )
        outputs.upToDateWhen { false }
        doFirst {
            val scenesDir = scenes.get().asFile
            val wireDir = wire.get().asFile
            for (dir in listOf(scenesDir, wireDir)) {
                dir.deleteRecursively()
                dir.mkdirs()
            }
            environment("HWUI_CONTRACT_DIR", scenesDir.absolutePath)
            environment("HWUI_KOTLIN_DIR", wireDir.absolutePath)
        }
    }

androidComponents {
    onVariants { variant ->
        variant.sources.kotlin?.addGeneratedSourceDirectory(contractScenes, ContractScenes::wire)
    }
}

tasks.withType<Test>().configureEach {
    dependsOn(contractScenes)
    systemProperty("hwui.contract.dir", contractScenes.flatMap { it.scenes }.get().asFile.absolutePath)
}
