package dev.waterui.hydrolysis

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.findViewTreeLifecycleOwner

/**
 * The mounted WaterUI app: the native session pointer plus the stable object
 * the Rust host bridge calls back on.
 *
 * The session object — not the view — is what JNI hands to the native side, so
 * a configuration change that recreates the [HydrolysisHostView] keeps the
 * same live native session; the new view binds to it through [bind]. The
 * owning ViewModel retains it across recreation and calls [destroy] when
 * its store is cleared; native teardown follows the host view's detach.
 */
class HydrolysisSession internal constructor(
    context: Context,
    internal var onCloseRequested: () -> Unit,
) {
    /**
     * The opaque native pointer, owned on the UI thread only. It is never
     * read directly: every Kotlin→native call takes it through
     * [withNativePtr], which refuses it once [destroy] has handed it to the
     * native side.
     */
    // A `ViewModel` retains the session, never an `Activity`: the native
    // side resolves the `Application` out of the context, so only the
    // application context crosses JNI.
    private val livePtr: Long =
        NativeBridge.nativeCreateSession(
            this,
            context.applicationContext,
            NativeBridge.uiThreadServices,
        )

    /** [destroy] ran; a second call is a named error. */
    private var destroyRequested = false

    /** The native teardown waits for the outermost native call to return. */
    private var tearDownPending = false

    /** [tearDown] has handed [livePtr] to `nativeDestroySession`. */
    private var destroyed = false

    /**
     * The Kotlin→native calls on the UI thread's stack. A native call can
     * call back into Kotlin, and that callback can make further native
     * calls, so the depth exceeds one; the native side holds the session
     * borrowed until the outermost call returns.
     */
    private var nativeDepth = 0

    /** Delivers the native requests that must not run inside a native call. */
    private val mainHandler = Handler(Looper.getMainLooper())

    /**
     * The session's frame pump. It belongs to the session, not to a host
     * view: a configuration change replaces the view but keeps this
     * session, and the one scheduler must stop with the native state it
     * drives — a view-owned scheduler outlives a replaced view and can
     * still hold a posted frame for a session destroyed later.
     */
    internal val frameScheduler: FrameScheduler = FrameScheduler(this)

    /**
     * Runs [block] with the native session pointer for the JNI entry named
     * [call] — the one accessor every Kotlin→native call that takes the
     * session goes through, host-family modules (the GPU band, painters)
     * included. One scope covers a sequence of calls that must reach the
     * same live session.
     *
     * From the moment [tearDown] hands the pointer to the native side, a
     * call that still arrives throws [IllegalStateException] naming
     * itself: a freed session is never reachable from Kotlin, so a late
     * call is a named error instead of a use-after-free. A teardown
     * requested while a scope is open — a callback the native side makes
     * mid-call that destroys the session and detaches its view — runs when
     * the outermost scope closes, never under a native frame that still
     * holds the session.
     */
    inline fun <R> withNativePtr(call: String, block: (ptr: Long) -> R): R {
        val ptr = enterNative(call)
        try {
            return block(ptr)
        } finally {
            exitNative()
        }
    }

    /** [withNativePtr]'s entry: refuses a destroyed session, then counts the call. */
    @PublishedApi
    internal fun enterNative(call: String): Long {
        check(!destroyed) { "hydrolysis: $call reached a destroyed HydrolysisSession" }
        nativeDepth += 1
        return livePtr
    }

    /** [withNativePtr]'s exit: the outermost return runs a pending teardown. */
    @PublishedApi
    internal fun exitNative() {
        nativeDepth -= 1
        if (nativeDepth == 0 && tearDownPending) tearDown()
    }

    /** The view currently presenting this session, or none between bindings. */
    internal var hostView: HydrolysisHostView? = null
        private set

    internal fun bind(view: HydrolysisHostView) {
        check(!destroyRequested) { "hydrolysis: bind reached a closing HydrolysisSession" }
        check(hostView == null || hostView === view) {
            "a HydrolysisSession is bound to exactly one host view at a time"
        }
        hostView = view
        // A session can bind after `onStart` already fired — a late mount
        // or a config-change rebind — and no later `setVisible` would ever
        // recover a false parked state. The mounted lifecycle's current
        // state is the truth; `onStart`/`onStop` keep updating it.
        setVisible(
            (view.findViewTreeLifecycleOwner() ?: view.context as? LifecycleOwner)?.lifecycle?.currentState
                ?.isAtLeast(Lifecycle.State.STARTED) == true,
        )
    }

    internal fun unbind(view: HydrolysisHostView) {
        if (hostView !== view) return
        hostView = null
        // A detached view is invisible, so park the pump before a pending
        // teardown runs below.
        if (!destroyed) setVisible(false)
        if (destroyRequested) requestTearDown()
    }

    /**
     * The owning lifecycle's started state (`onStart`/`onStop`). A stopped
     * session parks the frame pump — no frames, no Choreographer wakes —
     * until the next start; a start with a live surface renders exactly
     * the one current frame.
     */
    internal fun setVisible(visible: Boolean) {
        if (destroyRequested) return
        withNativePtr(NativeBridge::nativeSetVisible.name) { ptr ->
            NativeBridge.nativeSetVisible(ptr, visible)
        }
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
     * as soon as no native call is on the stack. Either way no host call
     * follows it.
     */
    internal fun destroy() {
        check(!destroyRequested) { "hydrolysis: HydrolysisSession.destroy() called twice" }
        setVisible(false)
        destroyRequested = true
        if (hostView?.isAttachedToWindow != true) requestTearDown()
    }

    /**
     * Runs [tearDown] now, or — inside a native call — when the outermost
     * one returns: the native frame on the stack still holds the session.
     */
    private fun requestTearDown() {
        if (nativeDepth == 0) tearDown() else tearDownPending = true
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
        check(nativeDepth == 0) { "hydrolysis: teardown ran inside a native call" }
        check(!destroyed) { "hydrolysis: HydrolysisSession torn down twice" }
        tearDownPending = false
        destroyed = true
        try {
            NativeBridge.nativeDestroySession(livePtr)
        } finally {
            mainHandler.removeCallbacksAndMessages(null)
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
        withNativePtr(NativeBridge::nativeBackEvent.name) { ptr ->
            NativeBridge.nativeBackEvent(ptr, phase, edge, progress)
        }
    }

    // ---- native → host callbacks (names are the JNI contract) ----

    /**
     * The scheduler belongs to the session, so a request made while no
     * view is bound — between a configuration change's unbind and the new
     * view's bind — still queues its frame.
     */
    @CalledFromNative
    fun onNativeRequestRedraw() {
        frameScheduler.requestFrame("redraw-request")
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

    /**
     * The native side requests a close from inside its frame and keeps
     * using the session after the request, so the close is posted: a
     * handler that destroys the session and detaches the view must run
     * after the frame returns, not under it. Teardown removes a close
     * still queued.
     */
    @CalledFromNative
    fun onNativeCloseRequested() {
        mainHandler.post { onCloseRequested() }
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
