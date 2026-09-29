package dev.waterui.hydrolysis

import android.os.Bundle
import android.view.View
import androidx.activity.ComponentActivity

/**
 * The minimal host activity: creates the [HydrolysisSession] once, mounts a
 * [HydrolysisHostView] on it, and keeps the session alive across configuration
 * recreation through `onRetainNonConfigurationInstance`.
 *
 * An app subclasses it and provides [nativeLibraryName] (its own Hydrolysis
 * cdylib) — the host library never picks a native library for the app, and the
 * schema handshake in [NativeBridge.load] catches a stale one.
 */
abstract class HydrolysisActivity : ComponentActivity() {

    /** The shared library the app's Hydrolysis runner lives in. */
    protected abstract val nativeLibraryName: String

    /** The mounted session; null before [onCreate] and after destroy. */
    protected var session: HydrolysisSession? = null
        private set

    /** The content view bound to [session]; override to add painter bands. */
    protected open fun createContentView(session: HydrolysisSession): View =
        HydrolysisHostView(this, session)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        NativeBridge.load(nativeLibraryName)
        @Suppress("DEPRECATION")
        val retained = lastCustomNonConfigurationInstance as? HydrolysisSession
        val session = retained ?: HydrolysisSession()
        this.session = session
        setContentView(createContentView(session))
    }

    @Deprecated("Configuration retention uses the platform non-configuration contract.")
    override fun onRetainCustomNonConfigurationInstance(): Any? = session

    override fun onDestroy() {
        // The session dies only with the activity — a configuration change
        // retains it, and the new host view binds to the same native state.
        if (isFinishing && !isChangingConfigurations) {
            session?.destroy()
        }
        session = null
        super.onDestroy()
    }
}
