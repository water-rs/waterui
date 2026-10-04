package dev.waterui.hydrolysis

import android.content.Context
import android.os.Build
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner

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
class HydrolysisSession internal constructor(context: Context) {
    /**
     * Opaque native pointer, owned on the UI thread only. Exposed to the
     * host-family modules (the GPU band, painters) that hand it back over JNI.
     */
    val nativePtr: Long =
        NativeBridge.nativeCreateSession(this, Build.VERSION.SDK_INT, context.applicationContext)

    /** The view currently presenting this session, or none between bindings. */
    internal var hostView: HydrolysisHostView? = null
        private set

    internal fun bind(view: HydrolysisHostView) {
        check(hostView == null || hostView === view) {
            "a HydrolysisSession is bound to exactly one host view at a time"
        }
        hostView = view
        // A session can bind after `onStart` already fired — a late mount
        // or a config-change rebind — and no later `setVisible` would ever
        // recover a false parked state. The lifecycle's current state is
        // the truth; `onStart`/`onStop` keep updating it from here.
        setVisible(
            (view.context as? LifecycleOwner)?.lifecycle?.currentState
                ?.isAtLeast(Lifecycle.State.STARTED) == true
        )
    }

    internal fun unbind(view: HydrolysisHostView) {
        if (hostView === view) hostView = null
    }

    /**
     * The owning Activity's started state (`onStart`/`onStop`). A stopped
     * session parks the frame pump — no frames, no Choreographer wakes —
     * until the next start; a start with a live surface renders exactly
     * the one current frame.
     */
    internal fun setVisible(visible: Boolean) {
        NativeBridge.nativeSetVisible(nativePtr, visible)
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
    fun onNativeAccessibilityTreeChanged(diffJson: String) {
        hostView?.notifyAccessibilityTreeChanged(diffJson)
    }

    @Suppress("unused") // called from native
    fun onNativePlatformViewsChanged() {
        hostView?.notifyPlatformViewsChanged()
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

    /** Native pushes the authoritative editing state for the IME mirror. */
    @Suppress("unused") // called from native
    fun onNativeEditingState(json: String) {
        hostView?.applyEditingState(json)
    }

    /** Native pushes a subscribed cursor-anchor update for the IME. */
    @Suppress("unused") // called from native
    fun onNativeCursorAnchorInfo(json: String) {
        hostView?.applyCursorAnchorInfo(json)
    }
}
