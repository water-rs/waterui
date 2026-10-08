package dev.waterui.hydrolysis

import android.os.Bundle
import android.view.View
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge

/**
 * The standalone host: the generated `MainActivity` subclasses it and gets a
 * WaterUI session for the app's `waterui_app` cdylib.
 *
 * The session lives in a `ViewModel` under this activity's store
 * ([HydrolysisEmbedding]), so it survives configuration changes; closing the
 * activity finishes it. Subclasses pick the painter through
 * [createContentView].
 */
abstract class HydrolysisActivity : ComponentActivity() {

    companion object {
        const val LOG_LEVEL_EXTRA = "waterui.log.level"
    }

    protected open val nativeLibraryName: String
        get() = BuildConfig.WATERUI_APP_LIBRARY

    /** The session's root view — the host view plus the painter's chrome. */
    protected abstract fun createContentView(session: HydrolysisSession): View

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContentView(
            HydrolysisEmbedding.createView(
                this,
                this,
                this,
                onBackPressedDispatcher,
                nativeLibraryName,
                onCloseRequested = ::finish,
                createContentView = ::createContentView,
                logLevel = intent.getStringExtra(LOG_LEVEL_EXTRA),
            ),
        )
    }
}
