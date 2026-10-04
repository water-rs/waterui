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

/**
 * Compose + Material 3 reference host for the Android parity E2E.
 *
 * Renders the twin registered for the example named by the `E2EExample`
 * intent extra — the same contract the SwiftUI reference host uses on the
 * Apple side. The shard driver launches this activity per example, captures
 * the settled frame, and compares it against the WaterUI render.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val example = intent.getStringExtra(EXTRA_EXAMPLE).orEmpty()
        val disableDynamic =
            intent.hasExtra("waterui.env.WATERUI_DISABLE_DYNAMIC_COLORS")
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
                Surface(modifier = Modifier.fillMaxSize().safeDrawingPadding()) {
                    val twin = twinFor(example)
                    if (twin != null) twin() else Text("No twin registered for example '$example'")
                }
            }
        }
    }

    companion object {
        const val EXTRA_EXAMPLE = "E2EExample"
        const val PACKAGE = "dev.waterui.android.reference"
    }
}
