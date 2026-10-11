// Compose reference applications for the section-6 benchmark harness.
//
// One statically selected application variant per fixture in the frozen
// inventory (bench/android/suite.toml), plus the `suite` flavor that roots
// every twin through the E2EExample intent extra — the same contract the
// imported android-backend reference used. Package comparisons are always
// per-fixture APK vs per-fixture APK (or suite vs suite); never a one-screen
// app against the rooted-suite APK.
//
// Build parity per section 6: release is non-debuggable, R8- and
// resource-shrunk, arm64-v8a only, with the shared toolchain pins recorded in
// bench/android/toolchain-lock.json and the campaign-frozen Compose BOM read
// from the `composeBomVersion` gradle property (see run.py lock-campaign).

import java.awt.Color
import java.awt.image.BufferedImage
import java.net.HttpURLConnection
import java.net.URI
import java.security.MessageDigest
import javax.imageio.ImageIO

// Frozen fixture inventory: every non-skip suite.toml entry, as a flavor
// name (suite.toml's `typography-rtl` is `typography_rtl` here — the
// registry maps flavor -> fixture by '_' -> '-').
val fixtureFlavors = listOf(
    "suite",
    "anchored_overlay", "animation", "drag_drop", "edge_layout", "edge_list",
    "edge_text", "filter", "flow_markdown", "form", "gallery", "gesture",
    "gradient", "hover", "icons", "list", "locale", "map", "markdown",
    "media_picker", "menu", "multi_window", "navigation", "picker",
    "reminders", "reply", "shape", "snackbar", "starfield", "stress",
    "typography_rtl", "video_player", "waterkit_camera_filters", "webview",
)

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
    id("androidx.baselineprofile")
}

android {
    namespace = "dev.waterui.android.reference"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.waterui.android.reference"
        minSdk = 31
        targetSdk = 37
        versionCode = 1
        versionName = "1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    flavorDimensions += "fixture"
    fixtureFlavors.forEach { name ->
        productFlavors.create(name) {
            dimension = "fixture"
            if (name != "suite") {
                applicationIdSuffix = ".$name"
            }
            buildConfigField(
                "String",
                "FIXTURE",
                "\"${name.replace('_', '-')}\"",
            )
        }
    }

    sourceSets {
        // Per-fixture variants compile only their own twin plus the small
        // fixtureContent() entry that selects it — R8's reachability then
        // prunes every other twin, which is what makes a per-fixture APK
        // size comparison honest. `suite` sees every twin so E2EExample can
        // dispatch to any screen.
        fixtureFlavors.filter { it != "suite" }.forEach { name ->
            named(name) {
                kotlin.directories.add("src/twins/$name/java")
                kotlin.directories.add("src/perfix/$name/java")
                assets.directories.add("src/twins/$name/assets")
            }
        }
        named("suite") {
            kotlin.directories.add("src/suite/java")
            layout.projectDirectory.dir("src/twins").asFile.listFiles()
                ?.filter { it.isDirectory }
                ?.forEach { dir ->
                    kotlin.directories.add(dir.resolve("java").path)
                    assets.directories.add(dir.resolve("assets").path)
                }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            isDebuggable = false
            isJniDebuggable = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
            // Benchmark builds are signed with the shared debug keystore —
            // identical signing across candidates is what section 6 requires.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a")
            isUniversalApk = false
        }
    }
    bundle {
        abi { enableSplit = false }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    packaging {
        resources {
            excludes += setOf(
                "META-INF/*.kotlin_module",
                "META-INF/DEPENDENCIES",
                "META-INF/AL2.0",
                "META-INF/LGPL2.1",
            )
        }
    }

    lint {
        abortOnError = true
        warningsAsErrors = true
        disable += listOf(
            "NewerVersionAvailable",
            "UnsafeOptInUsageError",
            // Per-fixture and suite variants legitimately keep different
            // assets and code reachable; unused-resource/resource-shrink
            // bookkeeping is R8's job.
            "UnusedResources",
        )
    }
}

// ---------------------------------------------------------------------------
// Bench assets are produced at build time: binary blobs never enter this
// repository's history.
//
//   bench_clip.mp4      The waterui video_player fixture's first remote
//                       sample ("Big Buck Bunny 1MB") fetched from the pinned
//                       upstream URL and verified by SHA-256 — the twin pins
//                       network content locally for measurements, so every
//                       source pill plays this clip.
//   tile_manhattan.png  Generated deterministically below: the waterui map
//                       fixture renders live vector tiles, so the twin draws
//                       a fixed stylized stand-in of the fixture's camera —
//                       Hudson/East River bands, the avenue (36 px period)
//                       and street (28 px period) grid, and Central Park.
// ---------------------------------------------------------------------------

val benchClipUrl =
    "https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/720/Big_Buck_Bunny_720_10s_1MB.mp4"
val benchClipSha256 =
    "18b99ec25f32f6bd2223aa54e4b5632533328bf5cc81c283eba7604c42649f75"

val videoPlayerAssetDir =
    objects.directoryProperty().apply {
        set(layout.buildDirectory.dir("generated/bench_assets/video_player"))
    }
val mapAssetDir =
    objects.directoryProperty().apply {
        set(layout.buildDirectory.dir("generated/bench_assets/map"))
    }

val fetchBenchClip =
    tasks.register("fetchBenchClip") {
        description = "Fetch the pinned Big Buck Bunny clip the video_player twin plays."
        group = "assets"
        outputs.dir(videoPlayerAssetDir)
        doLast {
            val target = videoPlayerAssetDir.get().file("bench_clip.mp4").asFile
            target.parentFile.mkdirs()
            val tmp = File(target.parentFile, "bench_clip.mp4.download")
            try {
                val connection =
                    URI(benchClipUrl).toURL().openConnection()
                        as HttpURLConnection
                connection.instanceFollowRedirects = true
                connection.connectTimeout = 30_000
                connection.readTimeout = 60_000
                connection.inputStream.use { stream ->
                    tmp.outputStream().use { stream.copyTo(it) }
                }
                val digest = MessageDigest.getInstance("SHA-256")
                tmp.inputStream().use { stream ->
                    val buffer = ByteArray(1 shl 20)
                    var read: Int
                    while (stream.read(buffer).also { read = it } >= 0) {
                        digest.update(buffer, 0, read)
                    }
                }
                val actual = digest.digest().joinToString("") { "%02x".format(it) }
                check(actual == benchClipSha256) {
                    "bench_clip.mp4 integrity check failed: expected sha256 " +
                        "$benchClipSha256, got $actual"
                }
                target.delete()
                check(tmp.renameTo(target)) {
                    "could not move $tmp into place as $target"
                }
            } finally {
                tmp.delete()
            }
        }
    }

val generateManhattanTile =
    tasks.register("generateManhattanTile") {
        description = "Draw the stylized Manhattan tile the map twin renders."
        group = "assets"
        outputs.dir(mapAssetDir)
        doLast {
            val water = Color(170, 211, 223).rgb
            val land = Color(233, 228, 220).rgb
            val sidewalk = Color(242, 239, 233).rgb
            val road = Color(255, 255, 255).rgb
            val park = Color(205, 234, 192).rgb

            val image =
                BufferedImage(
                    512,
                    512,
                    BufferedImage.TYPE_INT_RGB,
                )
            for (y in 0 until 512) {
                for (x in 0 until 512) {
                    image.setRGB(
                        x,
                        y,
                        when {
                            x < 70 || x > 452 -> water
                            x in 201..259 && y in 61..299 -> park
                            x % 36 < 3 || y % 28 < 3 -> road
                            x % 36 < 6 || y % 28 < 6 -> sidewalk
                            else -> land
                        },
                    )
                }
            }
            val file = mapAssetDir.get().file("tile_manhattan.png").asFile
            file.parentFile.mkdirs()
            ImageIO.write(image, "png", file)
        }
    }

androidComponents {
    onVariants { variant ->
        when (variant.productFlavors.singleOrNull()?.second) {
            "video_player", "suite" ->
                variant.sources.assets?.addGeneratedSourceDirectory(fetchBenchClip) {
                    videoPlayerAssetDir
                }
            "map" ->
                variant.sources.assets?.addGeneratedSourceDirectory(generateManhattanTile) {
                    mapAssetDir
                }
        }
        if (variant.productFlavors.singleOrNull()?.second == "suite") {
            variant.sources.assets?.addGeneratedSourceDirectory(generateManhattanTile) {
                mapAssetDir
            }
        }
    }
}

dependencies {
    // The campaign-frozen Compose BOM: run.py `lock-campaign` resolves the
    // current stable BOM at campaign start, records it and every resolved
    // dependency in bench/android/campaign-lock.json, and writes this
    // property so the value cannot drift mid-campaign.
    val bomVersion: String =
        providers.gradleProperty("composeBomVersion").getOrElse("2026.09.00")
    implementation(platform("androidx.compose:compose-bom:$bomVersion"))
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.profileinstaller:profileinstaller:1.4.1")
}
