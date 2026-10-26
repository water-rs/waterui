package dev.waterui.hydrolysis

import android.graphics.Canvas

/**
 * A painter that draws the session's top-level content in the host's own
 * `dispatchDraw`, interleaved with the embedded platform views in session
 * order, instead of through a band beneath them (the HWUI painter). Touches
 * follow the same order: the topmost entry under the pointer owns the
 * gesture, so a platform view drawn beneath session content does not receive
 * the touches that content covers.
 */
/**
 * Records one platform view's own display list into the painter's canvas —
 * its `RenderNode` portal inside the retained tree, so ancestor clip,
 * alpha and effects compose once.
 */
fun interface PlatformViewRecorder {
    /**
     * Draws the view registered under `id` into `canvas`, sized
     * `width`x`height` physical pixels with its world `transform` retained
     * for input mapping.
     */
    fun drawLocal(id: Long, canvas: Canvas, width: Int, height: Int, transform: android.graphics.Matrix)
}

interface OrderedContent {
    /** The entries, bottom first. */
    val entryCount: Int

    fun isPlatformView(index: Int): Boolean

    /** The placement id of platform-view entry `index`. */
    fun platformViewId(index: Int): Long

    /**
     * Re-records every retained platform-view portal before the host draws
     * — the recorder turns each binding into the native view's own display
     * list inside its owner's `RenderNode`.
     */
    fun preparePlatformViews(recorder: PlatformViewRecorder)

    /** Draws the window background, before any entry. */
    fun drawBackground(canvas: Canvas)

    /** Draws content entry `index` on the host's canvas. */
    fun drawEntry(canvas: Canvas, index: Int)

    /** Whether content entry `index` draws over host pixel (`x`, `y`). */
    fun covers(index: Int, x: Float, y: Float): Boolean

    /**
     * The platform view the retained tree places under host pixel (`x`, `y`)
     * — the authoritative answer for gesture ownership: ancestor alpha,
     * transforms, clips and scroll compose exactly as drawn. `null` means
     * self-drawn content or nothing covers the point.
     */
    fun platformViewAt(x: Float, y: Float): Long?
}

/** The ordered hit test, apart from views so the JVM tests it. */
internal object TouchOrder {
    /**
     * The topmost entry of `count` that `hits` claims, or -1 when none
     * does; `hits` is asked top first and stops at the first claim.
     */
    inline fun topmost(count: Int, hits: (Int) -> Boolean): Int {
        for (index in count - 1 downTo 0) {
            if (hits(index)) return index
        }
        return -1
    }
}
