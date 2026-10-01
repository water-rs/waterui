package dev.waterui.hydrolysis

import android.view.Surface

/**
 * JNI edge into the Hydrolysis runner. Every `external` name maps to a
 * `Java_dev_waterui_hydrolysis_NativeBridge_*` export; the schema handshake in
 * [load] refuses a native library built against a different edge so a stale
 * `.so` fails loudly instead of misreading arguments.
 */
object NativeBridge {
    /** Incremented in lock-step with `JNI_SCHEMA` in the Rust runner. */
    private const val SCHEMA: Int = 2

    private var initialized = false

    /**
     * Loads the app's Hydrolysis-backed shared library and verifies the JNI
     * schema. The application names its own `cdylib` — the host never picks
     * one for it.
     */
    @Synchronized
    fun load(libraryName: String) {
        if (initialized) return
        System.loadLibrary(libraryName)
        val nativeSchema = nativeInit(SCHEMA)
        check(nativeSchema == SCHEMA) {
            "hydrolysis JNI schema mismatch: host expects $SCHEMA, native library reports $nativeSchema"
        }
        initialized = true
    }

    @JvmStatic private external fun nativeInit(schema: Int): Int

    @JvmStatic
    external fun nativeCreateSession(session: HydrolysisSession, sdkInt: Int): Long

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

    @JvmStatic external fun nativeSetComposingText(sessionPtr: Long, text: String, caret: Int)

    @JvmStatic external fun nativeCommitText(sessionPtr: Long, text: String)

    @JvmStatic external fun nativeFinishComposingText(sessionPtr: Long)

    /** Serialized accesskit TreeUpdate, or null when nothing changed. */
    @JvmStatic external fun nativeAccessibilityTree(sessionPtr: Long): String?

    @JvmStatic
    external fun nativeAccessibilityAction(
        sessionPtr: Long,
        virtualViewId: Long,
        action: Int,
        value: String,
    ): Boolean

    /** Platform-view placement frame set, or null when unchanged. */
    @JvmStatic external fun nativePlatformViewFrames(sessionPtr: Long): String?
}
