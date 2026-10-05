package dev.waterui.hydrolysis

import android.os.Bundle
import android.view.View
import androidx.activity.BackEventCompat
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback

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
        NativeBridge.load(nativeLibraryName, intent.getStringExtra(LOG_LEVEL_EXTRA))
        @Suppress("DEPRECATION")
        val retained = lastCustomNonConfigurationInstance as? HydrolysisSession
        val session = retained ?: HydrolysisSession(this)
        this.session = session
        val backCallback = object : OnBackPressedCallback(false) {
            override fun handleOnBackStarted(backEvent: BackEventCompat) {
                session.dispatchBack(
                    NativeBridge.BACK_STARTED,
                    backEvent.swipeEdge,
                    backEvent.progress.toDouble(),
                )
            }

            override fun handleOnBackProgressed(backEvent: BackEventCompat) {
                session.dispatchBack(
                    NativeBridge.BACK_PROGRESSED,
                    backEvent.swipeEdge,
                    backEvent.progress.toDouble(),
                )
            }

            override fun handleOnBackCancelled() {
                session.dispatchBack(NativeBridge.BACK_CANCELLED, 0, 0.0)
            }

            override fun handleOnBackPressed() {
                session.dispatchBack(NativeBridge.BACK_INVOKED, 0, 0.0)
            }
        }
        onBackPressedDispatcher.addCallback(this, backCallback)
        session.onBackAvailable = { available -> backCallback.isEnabled = available }
        backCallback.isEnabled = session.backAvailable
        setContentView(createContentView(session))
    }

    @Deprecated("Configuration retention uses the platform non-configuration contract.")
    override fun onRetainCustomNonConfigurationInstance(): Any? = session

    override fun onStart() {
        super.onStart()
        session?.setVisible(true)
    }

    override fun onStop() {
        // The Android visibility signal: a stopped window's frame pump
        // parks — no frames, no Choreographer wakes — until `onStart`.
        session?.setVisible(false)
        super.onStop()
    }

    override fun onDestroy() {
        // Drop the callback before the session field goes: a retained session
        // must not keep writing `isEnabled` on the activity that is leaving.
        session?.onBackAvailable = null
        // The session dies only with the activity — a configuration change
        // retains it, and the new host view binds to the same native state.
        if (isFinishing && !isChangingConfigurations) {
            // A genuine finish commits the pending autofill save; a
            // configuration recreation keeps the session and drops nothing.
            session?.hostView?.autofillCommit()
            session?.destroy()
        }
        session = null
        super.onDestroy()
    }

    private companion object {
        /**
         * The launch-intent extra the CLI's `--logs` level arrives in
         * (`adb shell am start --es waterui.log.level debug`). It goes to the
         * native logging setup through [NativeBridge.load] — unlike the
         * `waterui.env.*` extras it never becomes an environment variable.
         */
        const val LOG_LEVEL_EXTRA = "waterui.log.level"
    }
}