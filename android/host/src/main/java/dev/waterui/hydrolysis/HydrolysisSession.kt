package dev.waterui.hydrolysis

import android.os.Build

/**
 * The mounted WaterUI app: the native session pointer plus the stable object
 * the Rust host bridge calls back on.
 *
 * The session object — not the view — is what JNI hands to the native side, so
 * a configuration change that recreates the [HydrolysisHostView] keeps the
 * same live native session; the new view binds to it through [bind]. The
 * activity that owns the session retains it across recreation and calls
 * [destroy] exactly once, when the process-side lifecycle truly ends.
 */
class HydrolysisSession internal constructor() {
    /**
     * Opaque native pointer, owned on the UI thread only. Exposed to the
     * host-family modules (the GPU band, painters) that hand it back over JNI.
     */
    val nativePtr: Long =
        NativeBridge.nativeCreateSession(this, Build.VERSION.SDK_INT)

    /** The view currently presenting this session, or none between bindings. */
    internal var hostView: HydrolysisHostView? = null
        private set

    internal fun bind(view: HydrolysisHostView) {
        check(hostView == null || hostView === view) {
            "a HydrolysisSession is bound to exactly one host view at a time"
        }
        hostView = view
    }

    internal fun unbind(view: HydrolysisHostView) {
        if (hostView === view) hostView = null
    }

    /** Tears down the native session. Idempotent guard lives in the caller. */
    fun destroy() {
        NativeBridge.nativeDestroySession(nativePtr)
    }

    // ---- native → host callbacks (names are the JNI contract) ----

    @Suppress("unused") // called from native
    fun onNativeRequestRedraw() {
        hostView?.requestFrame()
    }

    @Suppress("unused") // called from native
    fun onNativeTextInputState(x: Float, y: Float, width: Float, height: Float, purpose: Int) {
        hostView?.updateTextInputTarget(x, y, width, height, purpose)
    }

    @Suppress("unused") // called from native
    fun onNativeAccessibilityTreeChanged() {
        hostView?.notifyAccessibilityTreeChanged()
    }

    /**
     * A fatal, unrecoverable failure on the native side (GPU device loss, an
     * explicit GPU error). The host raises it as an exception on the UI
     * thread — a crash with a named cause is the honest surface for a session
     * that can no longer present.
     */
    @Suppress("unused") // called from native
    fun onNativeFatalError(message: String) {
        throw IllegalStateException(message)
    }

    @Suppress("unused") // called from native
    fun onNativeCloseRequested() {
        hostView?.closeRequested()
    }
}
