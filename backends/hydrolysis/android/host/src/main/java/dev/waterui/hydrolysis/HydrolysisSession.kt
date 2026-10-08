package dev.waterui.hydrolysis

import android.content.Context
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
 * [destroy] exactly once, when the process-side lifecycle truly ends; the
 * native teardown follows the host view's detach.
 */
class HydrolysisSession internal constructor(context: Context) {
    /**
     * The opaque native pointer, owned on the UI thread only. It is never
     * read directly: every Kotlin→native call takes it through [nativePtr],
     * which refuses it once [destroy] has handed it to the native side.
     */
    private val livePtr: Long =
        NativeBridge.nativeCreateSession(this, context, NativeBridge.uiThreadServices)

    /** [destroy] has handed [livePtr] to `nativeDestroySession`. */
    private var destroyed = false

    /**
     * [destroy] ran while the host view was still attached; the native
     * teardown waits for that view's detach.
     */
    private var destroyOnDetach = false

    /**
     * The session's frame pump. It belongs to the session, not to a host
     * view: a configuration change replaces the view but keeps this
     * session, and the one scheduler must stop with the native state it
     * drives — a view-owned scheduler outlives a replaced view and can
     * still hold a posted frame for a session destroyed later.
     */
    internal val frameScheduler: FrameScheduler = FrameScheduler(this)

    /**
     * The native session pointer for the JNI entry named [call] — the one
     * accessor every Kotlin→native call that takes the session goes
     * through, host-family modules (the GPU band, painters) included.
     *
     * From the moment [destroy] hands the pointer to the native side, a
     * call that still arrives throws [IllegalStateException] naming
     * itself: a freed session is never reachable from Kotlin, so a late
     * call is a named error instead of a use-after-free.
     */
    fun nativePtr(call: String): Long {
        check(!destroyed) { "hydrolysis: $call reached a destroyed HydrolysisSession" }
        return livePtr
    }

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
        if (hostView !== view) return
        hostView = null
        if (destroyOnDetach) tearDown()
    }

    /**
     * The owning Activity's started state (`onStart`/`onStop`). A stopped
     * session parks the frame pump — no frames, no Choreographer wakes —
     * until the next start; a start with a live surface renders exactly
     * the one current frame.
     */
    internal fun setVisible(visible: Boolean) {
        NativeBridge.nativeSetVisible(
            nativePtr(NativeBridge::nativeSetVisible.name),
            visible,
        )
    }

    /**
     * Ends the session, exactly once: a second call is a named error.
     *
     * An attached host view still forwards window events to the session
     * until its window is removed — an activity's `onDestroy`, or a
     * `ViewModel` cleared, runs before `ActivityThread` removes the window,
     * and the removal itself destroys the band's surface and releases
     * focused platform views. While a host view is attached the native
     * teardown therefore waits for its detach; with none attached it runs
     * now. Either way no host call follows it.
     */
    fun destroy() {
        check(!destroyOnDetach) { "hydrolysis: HydrolysisSession.destroy() called twice" }
        if (hostView?.isAttachedToWindow == true) {
            // A second destroy after the teardown is a named error too.
            nativePtr(NativeBridge::nativeDestroySession.name)
            destroyOnDetach = true
        } else {
            tearDown()
        }
    }

    /**
     * Frees the native session.
     *
     * The accessor closes before `nativeDestroySession` runs, so a host
     * call re-entering the session while its native state drops fails by
     * name instead of reaching the half-freed session. Teardown can still
     * request frames — releasing a focused platform view clears child
     * focus, and dropping reactive state asks for a redraw — so once the
     * native side returns, the frame scheduler stops: its posted callback
     * is removed and later requests post nothing. A panic during the
     * native teardown surfaces as [IllegalStateException] after both
     * steps have run.
     */
    private fun tearDown() {
        val ptr = nativePtr(NativeBridge::nativeDestroySession.name)
        destroyed = true
        try {
            NativeBridge.nativeDestroySession(ptr)
        } finally {
            frameScheduler.stop()
        }
    }

    /**
     * The latest "a navigation stack can accept back" answer. The activity
     * applies it when it attaches a callback, including after a configuration
     * change that retained this session: the native side reports only changes,
     * so a new activity cannot wait for another callback.
     */
    internal var backAvailable: Boolean = false
        private set

    /** The activity's back callback, or none between bindings. */
    internal var onBackAvailable: ((Boolean) -> Unit)? = null

    /** Forwards one system-back phase to the native session. */
    internal fun dispatchBack(phase: Int, edge: Int, progress: Double) {
        NativeBridge.nativeBackEvent(
            nativePtr(NativeBridge::nativeBackEvent.name),
            phase,
            edge,
            progress,
        )
    }

    // ---- native → host callbacks (names are the JNI contract) ----

    @CalledFromNative
    fun onNativeRequestRedraw() {
        hostView?.requestFrame()
    }

    @CalledFromNative
    fun onNativeSoftInput(visible: Boolean) {
        hostView?.setSoftInputVisible(visible)
    }

    @CalledFromNative
    fun onNativeAccessibilityTreeChanged(diffJson: String) {
        hostView?.notifyAccessibilityTreeChanged(diffJson)
    }

    @CalledFromNative
    fun onNativePlatformViewsChanged() {
        hostView?.notifyPlatformViewsChanged()
    }

    /**
     * A fatal, unrecoverable failure on the native side (GPU device loss, an
     * explicit GPU error). The host raises it as an exception on the UI
     * thread — a crash with a named cause is the honest surface for a session
     * that can no longer present.
     */
    @CalledFromNative
    fun onNativeFatalError(message: String) {
        throw IllegalStateException(message)
    }

    @CalledFromNative
    fun onNativeCloseRequested() {
        hostView?.closeRequested()
    }

    /** Native pushes the authoritative editing state for the IME mirror. */
    @CalledFromNative
    fun onNativeEditingState(json: String) {
        hostView?.applyEditingState(json)
    }

    /** Native pushes a subscribed cursor-anchor update for the IME. */
    @CalledFromNative
    fun onNativeCursorAnchorInfo(json: String) {
        hostView?.applyCursorAnchorInfo(json)
    }

    /**
     * The rendered frame's back-target answer changed. The activity enables
     * its back callback from this; a disabled callback leaves back to the
     * system, which finishes the activity.
     */
    @CalledFromNative
    fun onNativeBackAvailable(available: Boolean) {
        backAvailable = available
        onBackAvailable?.invoke(available)
    }
}
