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
 * ([HydrolysisEmbedding]), so it survives configuration changes and ends
 * only when the owner is cleared; closing the activity finishes it.
 * Subclasses name the library through [nativeLibraryName] and pick the
 * painter through [createContentView].
 */
abstract class HydrolysisActivity : ComponentActivity() {

    private companion object {
        const val LOG_LEVEL_EXTRA = "waterui.log.level"
    }

    /** The app's Hydrolysis launcher cdylib, loaded by [NativeBridge.load]. */
    protected abstract val nativeLibraryName: String

    /** The session's root view — the host view plus the painter's chrome. */
    protected abstract fun createContentView(session: HydrolysisSession): View

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // layout-spec.md §7.1: the app owns edge-to-edge presentation — the
        // system-bar and IME regions reach the renderer as insets the
        // layout engine reasons about, not as framework-consumed padding.
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
