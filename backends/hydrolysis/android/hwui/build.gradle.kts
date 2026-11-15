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
        minSdk = 34
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
        unitTests.all { it.jvmArgs("--add-opens=java.base/jdk.internal.access=ALL-UNNAMED") }
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
    implementation("androidx.core:core:1.17.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.robolectric:robolectric:4.17")
}

// Production wire generation is independent of contract-scene tests.
abstract class KotlinWire : Exec() {
    @get:OutputDirectory
    abstract val wire: DirectoryProperty
}

val repositoryRoot = rootDir.resolve("../../..")
val externalWire = providers.environmentVariable("WATERUI_HWUI_KOTLIN_DIR")
val kotlinWire = tasks.register<KotlinWire>("kotlinWire") {
    description = "Generates the Kotlin protocol through the normal Cargo build."
    wire.set(layout.buildDirectory.dir("generated/source/hwuiWire"))
    workingDir = repositoryRoot
    commandLine("python3", projectDir.resolve("../scripts/generate-hwui-wire.py"), wire.get().asFile)
    inputs.files(
        repositoryRoot.resolve("backends/hydrolysis/build.rs"),
        repositoryRoot.resolve("backends/hydrolysis/Cargo.toml"),
        repositoryRoot.resolve("backends/hydrolysis/src/hwui/codes.rs"),
        repositoryRoot.resolve("backends/hydrolysis/src/hwui/protocol.rs"),
        repositoryRoot.resolve("backends/hydrolysis/src/hwui/text/constants.rs"),
        repositoryRoot.resolve("backends/hydrolysis/src/hwui/kotlin.rs"),
        repositoryRoot.resolve("Cargo.lock"),
        projectDir.resolve("../scripts/generate-hwui-wire.py"),
    )
    inputs.dir(repositoryRoot.resolve("backends/hydrolysis/build"))
    inputs.dir(repositoryRoot.resolve("backends/hydrolysis/templates/hwui"))
}

abstract class ContractScenes : Exec() {
    @get:OutputDirectory
    abstract val scenes: DirectoryProperty
}

val contractScenes = tasks.register<ContractScenes>("contractScenes") {
    description = "Encodes the HWUI contract scenes for Kotlin replay tests."
    scenes.set(layout.buildDirectory.dir("contract"))
    workingDir = repositoryRoot
    commandLine(
        "cargo", "nextest", "run", "--locked", "-p", "hydrolysis",
        "-E", "test(hwui::contract) or test(hwui::kotlin)",
    )
    outputs.upToDateWhen { false }
    doFirst {
        val scenesDir = scenes.get().asFile
        scenesDir.deleteRecursively()
        scenesDir.mkdirs()
        environment("HWUI_CONTRACT_DIR", scenesDir.absolutePath)
    }
}

androidComponents {
    onVariants { variant ->
        if (externalWire.isPresent) {
            variant.sources.kotlin?.addStaticSourceDirectory(externalWire.get())
        } else {
            variant.sources.kotlin?.addGeneratedSourceDirectory(kotlinWire, KotlinWire::wire)
        }
    }
}

tasks.withType<Test>().configureEach {
    dependsOn(contractScenes)
    systemProperty("hwui.contract.dir", contractScenes.flatMap { it.scenes }.get().asFile.absolutePath)
}
