package {{ ctx.android_package_name() }}

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.system.Os
import android.util.Log
import android.view.View
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import dev.waterui.hydrolysis.HydrolysisActivity
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.HydrolysisSession
{%- if ctx.hydrolysis_android_has_painter_band() %}
import {{ ctx.hydrolysis_android_painter_band_import() }}
{%- endif %}

/**
 * The generated app entry: a [HydrolysisActivity] that loads this project's
 * Hydrolysis cdylib and mounts the painter's band beneath every host child.
 *
 * The host syncs the bundled asset tree off the main thread while the
 * launch screen stays up; the `waterui.env.*` intent extras the CLI passes
 * still override the sync's defaults, so they land in
 * [applyEnvironmentOverrides] — after the sync finishes, before the native
 * library loads.
 */
class MainActivity : HydrolysisActivity() {

    override val nativeLibraryName: String = "{{ ctx.hydrolysis_android_native_library_name() }}"

    override fun createContentView(session: HydrolysisSession): View {
        val context: Context = this
        return {% include "partials/hydrolysis_android_content_view.kt.tpl" %}
    }

    override fun applyEnvironmentOverrides() {
        setupEnvironmentFromIntent(intent)
    }

    /** Set once the session's root mounted; releases the launch screen. */
    private var sessionMounted = false

    /** The session's root mounted — the bench's cold-start marker. */
    override fun onSessionMounted() {
        sessionMounted = true
        Log.i(TAG, "WATERUI_ROOT_READY")
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // The launch screen the system showed from the tap on the icon stays
        // until the session mounts, so the window never shows the empty host
        // view the asset sync leaves in place.
        installSplashScreen().setKeepOnScreenCondition { !sessionMounted }

        super.onCreate(savedInstanceState)
    }

    companion object {
        private const val TAG = "WaterUI.MainActivity"
        private const val ENV_PREFIX = "waterui.env."

        /**
         * Read intent extras with prefix "waterui.env." and set them as environment variables.
         *
         * The CLI passes these extras via:
         * `adb shell am start ... --es waterui.env.<KEY> <VALUE>`
         *
         * This runs before loading native libraries so Rust can read env vars at startup.
         */
        private fun setupEnvironmentFromIntent(intent: Intent?) {
            val extras = intent?.extras ?: return

            for (key in extras.keySet()) {
                if (!key.startsWith(ENV_PREFIX)) continue

                val envVar = key.removePrefix(ENV_PREFIX)
                val value = extras.getString(key) ?: continue

                // A dev-server URL may only ever redirect a debuggable
                // build; a release APK always renders its staged bundle.
                if (envVar == "WATERUI_DEV_URL" && !BuildConfig.DEBUG) continue

                try {
                    Os.setenv(envVar, value, true)
                    Log.d(TAG, "Set environment variable $envVar from intent extra")
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to set environment variable $envVar: ${e.message}")
                }
            }
        }

    }
}
