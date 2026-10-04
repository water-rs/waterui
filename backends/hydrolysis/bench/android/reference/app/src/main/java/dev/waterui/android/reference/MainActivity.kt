package dev.waterui.android.reference

import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag

/**
 * Compose + Material 3 reference host for the Android benchmark harness.
 *
 * The suite variant renders the twin registered for the example named by the
 * `E2EExample` intent extra — the same contract the SwiftUI reference host
 * uses on the Apple side. Per-fixture variants statically select their twin
 * (see FixtureEntry.kt), so an E2EExample extra is not required to launch
 * them, and is ignored when present.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val example = intent.getStringExtra(EXTRA_EXAMPLE).orEmpty()
        val disableDynamic =
            intent.hasExtra("waterui.env.WATERUI_DISABLE_DYNAMIC_COLORS")
        BenchMarkers.enabled =
            intent.getBooleanExtra("waterui.bench.markers", false)
        // Environment contract shared with the Hydrolysis host: every
        // `waterui.env.*` extra becomes an environment value twins can read
        // (e.g. the stress counts suite.toml's env map sets).
        intent.extras?.let { extras ->
            for (key in extras.keySet()) {
                if (key.startsWith("waterui.env.")) {
                    AppEnv.values[key.removePrefix("waterui.env.")] =
                        extras.get(key)?.toString() ?: ""
                }
            }
        }
        setContent {
            // Match the runtime's theme source: WaterUiRootView wraps its
            // Material3 context in DynamicColors unless the e2e launch passes
            // the kill-switch extra — wallpaper-seeded palettes cannot back a
            // reproducible parity baseline.
            val dark = isSystemInDarkTheme()
            val scheme =
                if (!disableDynamic && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    if (dark) dynamicDarkColorScheme(this) else dynamicLightColorScheme(this)
                } else {
                    if (dark) darkColorScheme() else lightColorScheme()
                }
            MaterialTheme(colorScheme = scheme) {
                Surface(
                    modifier =
                        Modifier.fillMaxSize().safeDrawingPadding()
                            .testTag("screen-root"),
                ) {
                    val twin = fixtureContent(example)
                    if (twin != null) {
                        twin()
                    } else {
                        Text("No twin registered for example '$example'")
                    }
                }
            }
        }
    }

    companion object {
        const val EXTRA_EXAMPLE = "E2EExample"
        const val PACKAGE = "dev.waterui.android.reference"
    }
}

/** Environment values delivered through `waterui.env.*` launch extras. */
object AppEnv {
    val values = mutableMapOf<String, String>()

    fun get(name: String): String? = values[name]

    fun int(name: String, fallback: Int): Int =
        values[name]?.toIntOrNull() ?: fallback
}
