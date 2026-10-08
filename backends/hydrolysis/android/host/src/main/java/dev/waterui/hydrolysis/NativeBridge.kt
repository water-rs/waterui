package dev.waterui.hydrolysis

import android.content.Context
import android.view.Surface

/**
 * JNI edge into the Hydrolysis runner. Every `external` name maps to a
 * `Java_dev_waterui_hydrolysis_NativeBridge_*` export; the schema handshake in
 * [load] refuses a native library built against a different edge so a stale
 * `.so` fails loudly instead of misreading arguments.
 */
object NativeBridge {
    /**
     * Incremented in lock-step with `JNI_SCHEMA` in the Rust runner.
     * History: 1 = surface/input/IME events; 2 = `nativeSetVisible`
     * (Activity `onStart`/`onStop` → pump visibility); 3 = the
     * InputConnection range protocol (`nativeEditOp`/`nativeEditingState`,
     * the `onNative*` editing pushes) and the `Context` handed to
     * [nativeCreateSession]; 4 = `nativeAccessibilityAction` takes the
     * accesskit action index plus selection-bounds, text and numeric
     * payload channels; 5 = `onNativeAccessibilityTreeChanged` carries the
     * diffed event-list JSON and [nativeAccessibilityHitTest] maps a point
     * to the served virtual node for explore-by-touch; 6 = [nativeInit]
     * carries the launch intent's `waterui.log.level` extra (the CLI's
     * `--logs` level) and logging init moves out of the app cdylib's
     * `JNI_OnLoad`; 7 = `nativeSetMetrics` carries the `ViewConfiguration`
     * touch-scroll parameters (slop, min/max fling velocity, scroll
     * friction); 8 = [nativeCreateSession] drops `sdkInt`. The API floor is
     * 31, so `ANativeWindow_setFrameRate` is linked directly; 9 =
     * [nativeBackEvent] and `onNativeBackAvailable` carry system back into
     * the navigation stack and report whether a back target is registered;
     * 10 = `nativeSetMetrics` splits the window insets into the container
     * and keyboard regions of layout-spec.md §7.1, and the host's
     * `WindowInsetsAnimationCompat` progress pushes each IME animation frame;
     * 11 = [nativeUiThreadServices] creates the one executor per UI thread
     * at load time, and [nativeCreateSession] takes its handle so every
     * session shares it.
     */
    private const val SCHEMA: Int = 11

    /** [nativeBackEvent] phase: a predictive gesture began. */
    const val BACK_STARTED: Int = 0

    /** [nativeBackEvent] phase: the gesture's progress, in `0..1`. */
    const val BACK_PROGRESSED: Int = 1

    /** [nativeBackEvent] phase: the platform cancelled the gesture. */
    const val BACK_CANCELLED: Int = 2

    /** [nativeBackEvent] phase: the gesture committed, or a back button was pressed. */
    const val BACK_INVOKED: Int = 3

    private var initialized = false

    /**
     * Opaque handle for the UI-thread services the load hook created once:
     * the executor every session shares, registered with the main looper.
     * Sessions receive it explicitly — the native side keeps no global.
     */
    internal var uiThreadServices: Long = 0
        private set

    /**
     * Loads the app's Hydrolysis-backed shared library and verifies the JNI
     * schema. The application names its own `cdylib` — the host never picks
     * one for it. `logLevel` is the `tracing` level name the launch intent's
     * `waterui.log.level` extra carried, or null when the launch asked for
     * no level — the native logging setup keeps its own default then.
     */
    @Synchronized
    fun load(libraryName: String, logLevel: String?) {
        if (initialized) return
        System.loadLibrary(libraryName)
        val nativeSchema = nativeInit(SCHEMA, logLevel)
        check(nativeSchema == SCHEMA) {
            "hydrolysis JNI schema mismatch: host expects $SCHEMA, native library reports $nativeSchema"
        }
        uiThreadServices = nativeUiThreadServices()
        check(uiThreadServices != 0L) { "hydrolysis: the UI-thread executor was not created" }
        initialized = true
    }

    @JvmStatic private external fun nativeInit(schema: Int, logLevel: String?): Int

    /**
     * Creates the one executor per UI thread — registered with the main
     * looper through the same eventfd it writes — plus the process
     * environment carrying the inspector. Called once from [load], which is
     * why the executor exists for the process's life rather than per
     * session. Returns the process-owned handle passed to
     * [nativeCreateSession].
     */
    @JvmStatic private external fun nativeUiThreadServices(): Long

    /**
     * `context` is any `Context` of the app — the native side resolves its
     * `Application` and publishes that, once per process, through
     * `ndk_context` so service backends (clipboard) can resolve it.
     * `uiThreadServices` is the handle [load] created — every session
     * shares that executor rather than installing its own.
     */
    @JvmStatic
    external fun nativeCreateSession(
        session: HydrolysisSession,
        context: Context,
        uiThreadServices: Long,
    ): Long

    @JvmStatic external fun nativeDestroySession(sessionPtr: Long)

    @JvmStatic
    external fun nativeSetMetrics(
        sessionPtr: Long,
        widthPx: Int,
        heightPx: Int,
        density: Float,
        fontScale: Float,
        refreshHz: Float,
        insetLeft: Int,
        insetTop: Int,
        insetRight: Int,
        insetBottom: Int,
        imeLeft: Int,
        imeTop: Int,
        imeRight: Int,
        imeBottom: Int,
        touchSlopPx: Float,
        minFlingVelocityPx: Float,
        maxFlingVelocityPx: Float,
        scrollFriction: Float,
    )

    /**
     * The frame transaction. Returns a bitmask: 1 = wants another vsync frame,
     * 2 = window requested close, 4 = a fallback deadline is available through
     * [nativeFrameDeadlineInNanos].
     */
    @JvmStatic external fun nativeOnFrame(sessionPtr: Long, vsyncNanos: Long): Long

    @JvmStatic external fun nativeFrameDeadlineInNanos(sessionPtr: Long): Long

    @JvmStatic
    external fun nativeSurfaceAttached(
        sessionPtr: Long,
        surface: Surface,
        width: Int,
        height: Int,
        generation: Long,
    ): Boolean

    @JvmStatic
    external fun nativeSurfaceChanged(
        sessionPtr: Long,
        width: Int,
        height: Int,
        generation: Long,
    )

    @JvmStatic external fun nativeSurfaceDestroyed(sessionPtr: Long, generation: Long)

    /**
     * The Activity's started state (`onStart`/`onStop`) — drives the frame
     * pump's hidden flag together with the surface's attach/detach.
     */
    @JvmStatic external fun nativeSetVisible(sessionPtr: Long, visible: Boolean)

    @JvmStatic external fun nativeSetHighRefresh(sessionPtr: Long, fps: Float)

    /**
     * One system-back phase. `phase` is a `BACK_*` constant. `edge` is
     * [androidx.activity.BackEventCompat.EDGE_LEFT] or `EDGE_RIGHT` and is
     * read only for [BACK_STARTED]. `progress` is the platform's `0..1` report.
     */
    @JvmStatic
    external fun nativeBackEvent(sessionPtr: Long, phase: Int, edge: Int, progress: Double)

    @JvmStatic
    external fun nativePointerEvent(
        sessionPtr: Long,
        action: Int,
        pointerId: Int,
        x: Float,
        y: Float,
        toolType: Int,
        button: Int,
    )

    @JvmStatic
    external fun nativeScrollEvent(sessionPtr: Long, x: Float, y: Float, dx: Float, dy: Float)

    @JvmStatic
    external fun nativeKeyEvent(
        sessionPtr: Long,
        key: String,
        pressed: Boolean,
        shift: Boolean,
        ctrl: Boolean,
        alt: Boolean,
        meta: Boolean,
    )

    /**
     * One `InputConnection` mutator, dispatched onto the session's editing
     * state machine. `op` is an `EDIT_OP_*` constant from
     * [HydrolysisInputConnection]; `text` is the CharSequence argument where
     * the op carries one. Returns false for a stale `editorId` (a connection
     * whose editor lost focus) — the mirror must not move then.
     */
    @JvmStatic
    external fun nativeEditOp(
        sessionPtr: Long,
        editorId: Long,
        op: Int,
        arg1: Int,
        arg2: Int,
        text: String,
    ): Boolean

    /**
     * The authoritative editing state as JSON, pulled synchronously when a
     * connection binds — its `editor_id` is the connection's generation
     * token. Pushes arrive later as `onNativeEditingState`.
     */
    @JvmStatic external fun nativeEditingState(sessionPtr: Long): String?

    /** Serialized accesskit TreeUpdate, or null when nothing changed. */
    @JvmStatic external fun nativeAccessibilityTree(sessionPtr: Long): String?

    /**
     * The served virtual node under (`x`, `y`) in logical units, or -1 —
     * the hit test the host's `dispatchHoverEvent` consults for
     * explore-by-touch.
     */
    @JvmStatic
    external fun nativeAccessibilityHitTest(
        sessionPtr: Long,
        x: Float,
        y: Float,
    ): Long

    /**
     * An accessibility action on a virtual node. `action` is the *accesskit*
     * action index (not an Android constant); `arg1`/`arg2` carry the
     * `SetTextSelection` UTF-16 bounds (-1 = none), `text` the `SetValue`/
     * `ReplaceSelectedText` string (empty = none), and `numeric` the
     * `NumericValue`/`CustomAction` payload (NaN = none). Text actions on an
     * editable node run over the session's editing protocol — the same
     * writer the `InputConnection` mirror uses.
     */
    @JvmStatic
    external fun nativeAccessibilityAction(
        sessionPtr: Long,
        virtualViewId: Long,
        action: Int,
        arg1: Int,
        arg2: Int,
        text: String,
        numeric: Double,
    ): Boolean

    /** Platform-view placement frame set, or null when unchanged. */
    @JvmStatic external fun nativePlatformViewFrames(sessionPtr: Long): String?
}
