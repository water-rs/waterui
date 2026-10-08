package dev.waterui.hydrolysis

import android.content.Context
import android.view.View
import dev.waterui.hydrolysis.MutableContextWrapper
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
        NativeBridge.nativeCreateSession(this, context, NativeBridge.uiThreadServices)

    /** The view currently presenting this session, or none between bindings. */
    internal var hostView: HydrolysisHostView? = null
        private set

    /**
     * The context of the bound host view — the JNI-side contract the
     * platform-view instance modules create their views with (the `webview`
     * module wraps it in a `MutableContextWrapper`). `null` while unbound.
     */
    val boundContext: Context?
        get() = hostView?.context

    /**
     * The platform-view *instances* this session owns: native views the Rust
     * side constructed over JNI — the system `WebView` — and registers here
     * so a placement naming `{"instance": id}` can mount them. The JNI-side
     * contract: `HydrolysisWebView.create` calls
     * [registerPlatformViewInstance] and `release()` calls
     * [unregisterPlatformViewInstance].
     */
    private val platformViewInstances = HashMap<Long, View>()

    /**
     * Registers `view` as the platform-view instance `id`. Public because the
     * registering code lives in another Gradle module.
     */
    fun registerPlatformViewInstance(id: Long, view: View) {
        check(platformViewInstances.putIfAbsent(id, view) == null) {
            "hydrolysis: a platform-view instance is already registered under id $id"
        }
        // A placement naming this instance may already be waiting.
        hostView?.platformViewRegistry?.notifyChanged()
    }

    /**
     * Drops instance `id` from the registry; any slot currently holding it
     * goes with it.
     */
    fun unregisterPlatformViewInstance(id: Long) {
        platformViewInstances.remove(id)
        hostView?.platformViewRegistry?.onInstanceUnregistered(id)
    }

    /** The instance a placement resolves, or none. Registry-side. */
    internal fun platformViewInstance(id: Long): View? = platformViewInstances[id]

    internal fun bind(view: HydrolysisHostView) {
        check(hostView == null || hostView === view) {
            "a HydrolysisSession is bound to exactly one host view at a time"
        }
        hostView = view
        // The new binding owns the Activity the instances draw on: retarget
        // their `MutableContextWrapper`s, and let each re-observe what the
        // old binding's context owned (the WebView's Lifecycle).
        for (instance in platformViewInstances.values) {
            (instance.context as? MutableContextWrapper)?.baseContext = view.context
            (instance as? HostRebindAware)?.onHostRebound(view.context)
        }
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
        if (hostView === view) {
            // The registry dies with the view; the instances outlive it on a
            // retained session, so they must be free of the old slots before
            // the next binding mounts them.
            view.platformViewRegistry.detachInstances()
            hostView = null
        }
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
        NativeBridge.nativeBackEvent(nativePtr, phase, edge, progress)
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
