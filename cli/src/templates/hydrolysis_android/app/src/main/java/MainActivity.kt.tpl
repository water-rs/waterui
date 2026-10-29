package {{ ctx.android_package_name() }}

import android.content.Context
import android.os.Bundle
import android.util.Log
import android.view.View
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import dev.waterui.hydrolysis.HydrolysisActivity
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.LaunchEnvironmentLoader
{%- if ctx.hydrolysis_android_is_hwui() %}
import dev.waterui.hydrolysis.SessionPainter
import dev.waterui.hydrolysis.hwui.HwuiPainter
{%- endif %}
{%- if ctx.hydrolysis_android_has_painter_band() %}
import {{ ctx.hydrolysis_android_painter_band_import() }}
{%- endif %}

/**
 * The generated app entry: a [HydrolysisActivity] that loads this project's
 * Hydrolysis cdylib and mounts the painter's band beneath every host child.
 *
 * The host syncs the bundled asset tree off the main thread while the
 * launch screen stays up; the launch-environment handoff the CLI staged
 * still overrides the sync's defaults, so its claim lands in
 * [prepareEnvironmentOverrides] — off the main thread, before the native
 * library loads.
 */
class MainActivity : HydrolysisActivity() {

    override val nativeLibraryName: String = "{{ ctx.hydrolysis_android_native_library_name() }}"
{%- if ctx.hydrolysis_android_is_hwui() %}

    override fun createPainter(): SessionPainter = HwuiPainter()
{%- endif %}

    override fun createContentView(session: HydrolysisSession): View {
        val context: Context = this
        return {% include "partials/hydrolysis_android_content_view.kt.tpl" %}
    }

    override fun prepareEnvironmentOverrides(callback: (() -> Unit) -> Unit) {
        LaunchEnvironmentLoader(this).prepare(
            intent.getStringExtra(ENVIRONMENT_ID_EXTRA),
            callback,
        )
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

        /**
         * The launch intent's handoff id. The environment values it routes
         * never travel the intent — they reach the app through the private
         * `LaunchEnvironmentProvider` the CLI writes before `am start`.
         */
        private const val ENVIRONMENT_ID_EXTRA = "waterui.launch.environment"
    }
}
