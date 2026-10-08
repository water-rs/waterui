package {{ ctx.android_package_name() }}

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.system.Os
import android.util.Log
import android.view.View
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import dev.waterui.hydrolysis.HydrolysisEnvironment
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
 * [HydrolysisEnvironment.prepare] owns the bundled asset tree and the default
 * environment variables; the intent extras the CLI passes still override the
 * defaults it sets, so they land between it and `super.onCreate` (which loads
 * the native library).
 */
class MainActivity : HydrolysisActivity() {

    override val nativeLibraryName: String = "{{ ctx.hydrolysis_android_native_library_name() }}"

    override fun createContentView(session: HydrolysisSession): View {
        val context: Context = this
        return {% include "partials/hydrolysis_android_content_view.kt.tpl" %}
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // The launch screen the system showed from the tap on the icon stays
        // until this activity's first frame.
        installSplashScreen()

        // super.onCreate loads the native library and registers the app, so
        // every environment hand-off lands before it.
        HydrolysisEnvironment.prepare(this)
        setupEnvironmentFromIntent(intent)

        super.onCreate(savedInstanceState)
        Log.i(TAG, "WATERUI_ROOT_READY")
    }

    override fun onDestroy() {
        val activityFinished = isFinishing && !isChangingConfigurations
        super.onDestroy()
        if (activityFinished) {
            Log.i(TAG, "WATERUI_ACTIVITY_FINISHED")
        }
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
