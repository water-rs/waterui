package dev.waterui.hydrolysis

import android.content.Context
import android.content.res.Configuration
import android.os.Handler
import android.os.Looper
import android.view.View
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
    private val onCloseRequested: () -> Unit,
    private val painter: SessionPainter? = null,
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
            painter,
            NativeBridge.uiThreadServices,
            context.resources.configuration.isNightMode,
        )

    /** [destroy] ran; a second call is a named error. */
    private var destroyRequested = false

    /** The native teardown waits for the outermost native call to return. */
    private var tearDownPending = false

    /** A placement pull requested under a native call; the outermost [exitNative] runs it. */
    private var placementPullPending = false

    /**
     * A soft-input request made under a native call; the outermost
     * [exitNative] applies the latest one — a later request replaces an
     * earlier one inside the same outer call.
     */
    private var softInputPending: Boolean? = null

    /**
     * A [translucency] change made under a native call; the outermost
     * [exitNative] delivers it to the listeners.
     */
    private var translucencyPending = false

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
    internal val frameScheduler: FrameScheduler = FrameScheduler(
        this,
        if (painter == null && android.os.Build.VERSION.SDK_INT >= 35) FramePerformance(context) else null,
    )

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

    /**
     * [withNativePtr]'s exit: the outermost return runs a pending teardown,
     * else a pending placement pull, then the latest pending soft-input
     * request and translucency change. The pull opens its own [withNativePtr]
     * scope, so this runs again at depth zero with the flag already cleared;
     * a pending teardown wins and drops every deferred request.
     */
    @PublishedApi
    internal fun exitNative() {
        nativeDepth -= 1
        if (nativeDepth != 0) return
        if (tearDownPending) {
            placementPullPending = false
            softInputPending = null
            translucencyPending = false
            tearDown()
        } else {
            if (placementPullPending) {
                placementPullPending = false
                hostView?.platformViewRegistry?.publishIfPending()
            }
            softInputPending?.let { visible ->
                softInputPending = null
                hostView?.setSoftInputVisible(visible)
            }
            if (translucencyPending) {
                translucencyPending = false
                translucencyListeners.forEach { it(translucency) }
            }
        }
    }

    /**
     * Pulls the published placement set into the bound registry, never under a
     * native call: the native side holds the session borrowed until the
     * outermost call returns, and `nativePlatformViewFrames` would re-enter it.
     */
    internal fun requestPlacementPull() {
        if (nativeDepth == 0) hostView?.platformViewRegistry?.publishIfPending()
        else placementPullPending = true
    }

    /** The view currently presenting this session, or none between bindings. */
    internal var hostView: HydrolysisHostView? = null
        private set

    /**
     * The context of the bound host view — the JNI-side contract the
     * platform-view instance modules create their views with (the `webview`
     * module wraps it in an `android.content.MutableContextWrapper`).
     * `null` while unbound.
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
     * Registers `view` as the platform-view instance `id` and requests a
     * placement pull (deferred while a native call is live, see
     * [requestPlacementPull]). Public because the registering code lives in
     * another Gradle module.
     */
    fun registerPlatformViewInstance(id: Long, view: View) {
        check(platformViewInstances.putIfAbsent(id, view) == null) {
            "hydrolysis: a platform-view instance is already registered under id $id"
        }
        // A placement naming this instance may already be waiting.
        requestPlacementPull()
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

    /**
     * The retained tree's platform view under host pixel (`x`, `y`) — the
     * HWUI painter's ordered hit test; `null` means no view is drawn there
     * or an interactive target above it takes the point. GPU sessions never
     * call this.
     */
    internal fun platformViewAt(x: Float, y: Float): Long? {
        val id =
            withNativePtr(NativeBridge::nativePlatformViewHitTest.name) { ptr ->
                NativeBridge.nativePlatformViewHitTest(ptr, x, y)
            }
        return if (id == 0L) null else id
    }

    internal fun bind(view: HydrolysisHostView) {
        check(!destroyRequested) { "hydrolysis: bind reached a closing HydrolysisSession" }
        check(hostView == null || hostView === view) {
            "a HydrolysisSession is bound to exactly one host view at a time"
        }
        hostView = view
        painter?.attach(view)
        withNativePtr(NativeBridge::nativeSetHostAttached.name) { ptr ->
            NativeBridge.nativeSetHostAttached(ptr, true)
        }
        // A session retained across an activity recreation binds its new
        // view under the new configuration — the recreate path delivers no
        // `onConfigurationChanged`, so the bind itself reports the scheme.
        setNightMode(view.resources.configuration.isNightMode)
        // The new binding owns the Activity the instances draw on: each
        // `HostRebindAware` retargets its own context wrapper and re-observes
        // what the old binding's context owned (the WebView's Lifecycle).
        for (instance in platformViewInstances.values) {
            (instance as? HostRebindAware)?.onHostRebound(view.context)
        }
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
        // The registry dies with the view; the instances outlive it on a
        // retained session, so they must be free of the old slots before
        // the next binding mounts them.
        view.platformViewRegistry.detachInstances()
        painter?.detach(view)
        if (!destroyed) {
            withNativePtr(NativeBridge::nativeSetHostAttached.name) { ptr ->
                NativeBridge.nativeSetHostAttached(ptr, false)
            }
        }
        hostView = null
        // A detached view is invisible, so park the pump before a pending
        // teardown runs below.
        if (!destroyed) setVisible(false)
        if (destroyRequested) requestTearDown()
    }

    /**
     * Routes the scheduler's frame demand: a session-owned painter applies
     * it on the bound host; the GPU path reports it to native, which
     * combines it with the engine's content-rate range.
     */
    internal fun setFrameDemand(continuous: Boolean, touching: Boolean): Float {
        check(!destroyed) { "hydrolysis: frame demand reached a destroyed HydrolysisSession" }
        return if (painter != null) {
            painter.setHighRefresh(continuous || touching)
            0f
        } else {
            withNativePtr(NativeBridge::nativeSetFrameDemand.name) { ptr ->
                NativeBridge.nativeSetFrameDemand(ptr, continuous, touching)
            }
        }
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
     * The configuration's night-mode flag, into the session's colour-scheme
     * signal — the scheme the theme tokens resolve against. The host calls
     * this at bind and on every `onConfigurationChanged`; an unchanged value
     * re-resolves nothing native-side.
     */
    internal fun setNightMode(nightMode: Boolean) {
        if (destroyRequested) return
        withNativePtr(NativeBridge::nativeSetColorScheme.name) { ptr ->
            NativeBridge.nativeSetColorScheme(ptr, nightMode)
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
            try {
                painter?.destroy()
            } finally {
                mainHandler.removeCallbacksAndMessages(null)
                frameScheduler.stop()
            }
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

    /**
     * The window's compositing request the native side last reported:
     * [translucent] composites the window and its surface with what lies
     * behind it, [blurBehind] asks the platform to blur that content — a
     * behind-window material implies both. Retained across host bindings so
     * a view rebound after a configuration change applies it without waiting
     * for another push.
     */
    var translucency: Translucency = Translucency(translucent = false, blurBehind = false, blurRadiusPx = 0)
        private set

    /** The compositing request the native side pushes through [onNativeTranslucency]. */
    data class Translucency(val translucent: Boolean, val blurBehind: Boolean, val blurRadiusPx: Int)

    /**
     * The objects a change of [translucency] must reach: the bound host view
     * applies the window and Activity half, the GPU band its surface format.
     * A listener is fired with the current request as it registers, so a
     * late mount never waits for another push.
     */
    private val translucencyListeners = java.util.concurrent.CopyOnWriteArrayList<(Translucency) -> Unit>()

    /**
     * Registers [listener] for the window's compositing requests, fired
     * immediately with the current one. Public because the GPU band lives in
     * another Gradle module.
     */
    fun addTranslucencyListener(listener: (Translucency) -> Unit) {
        translucencyListeners.add(listener)
        listener(translucency)
    }

    /** Drops [listener] — a host view detaches its listener when its window goes away. */
    fun removeTranslucencyListener(listener: (Translucency) -> Unit) {
        translucencyListeners.remove(listener)
    }

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

    /**
     * The native side asks for the soft keyboard from inside its own call.
     * `showSoftInput` builds the input connection synchronously, and that
     * connection's `nativeEditingState` pull would re-enter the session the
     * call still holds borrowed — so the request defers to the outermost
     * [exitNative] like a placement pull does, the latest request winning.
     * Made at depth zero, it reaches the host view at once.
     */
    @CalledFromNative
    fun onNativeSoftInput(visible: Boolean) {
        if (nativeDepth == 0) hostView?.setSoftInputVisible(visible)
        else softInputPending = visible
    }

    @CalledFromNative
    fun onNativeAccessibilityTreeChanged(diffJson: String) {
        hostView?.notifyAccessibilityTreeChanged(diffJson)
    }

    /**
     * The session published a new placement set. Fired from inside the frame,
     * so the pull runs once the outermost native call returns.
     */
    @CalledFromNative
    fun onNativePlatformViewsChanged() {
        requestPlacementPull()
    }

    /**
     * The one focus query the frame makes: whether a mounted platform-view
     * child holds UI focus. A pull, never a push — the focus listener would
     * re-enter the session synchronously from `requestFocus`/`removeView`.
     */
    @CalledFromNative
    fun onNativePlatformViewFocus(): Boolean =
        hostView?.platformViewRegistry?.focusInside() == true

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

    /**
     * The resolved window background's compositing request. Carries the same
     * answer the surface's compositor planes already render by: translucent
     * asks the window and surface to let behind-window content show, and a
     * behind-window material asks the platform to blur it.
     *
     * The native side reports it from inside its frame. The listeners switch
     * the Activity window and recreate the band's surface, whose callbacks
     * call back into the session the frame still holds borrowed, so a change
     * made under a native call reaches them from the outermost [exitNative],
     * like a soft-input request.
     */
    @CalledFromNative
    fun onNativeTranslucency(translucent: Boolean, blurBehind: Boolean, blurRadiusPx: Int) {
        val next = Translucency(translucent, blurBehind, blurRadiusPx)
        if (next == translucency) return
        translucency = next
        if (nativeDepth == 0) translucencyListeners.forEach { it(next) }
        else translucencyPending = true
    }
}

/**
 * The configuration's night-mode flag (`UI_MODE_NIGHT_YES` under
 * `UI_MODE_NIGHT_MASK`) — the colour-scheme input the host reports to the
 * session at creation, at bind and on every configuration change.
 */
internal val Configuration.isNightMode: Boolean
    get() = uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
