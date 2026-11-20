package dev.waterui.hydrolysis

import android.content.Context
import android.graphics.ColorSpace
import android.widget.FrameLayout

/**
 * A band the [PlatformViewRegistry] stacks right after an ordered platform
 * view: a [FrameLayout] container whose drawn slices show only the self-drawn
 * regions a frame paints above that view and beneath the next one
 * (water-rs/waterui#2225). Being a View in paint order, it composites between
 * the platform views the way the retained tree orders them, while covering only
 * painted regions for ad viewability.
 *
 * The painter supplies the realization through
 * [PlatformViewRegistry.bandFactory]; the session's [show] and [clear]
 * calls tell it a frame's image is complete. Both are called on the UI
 * thread ahead of the next traversal, so the band draws the change in the
 * same view frame as every other band the frame changed.
 */
abstract class RegionBand(context: Context) : FrameLayout(context) {
    /**
     * Shows the newest image the band's producer delivered, its pixels
     * encoded in [colorSpace] — the colour space the producer rendered it in —
     * across the packed `[x, y, width, height, ...]` device-pixel rectangles
     * [paintedRects].
     */
    abstract fun show(colorSpace: ColorSpace, paintedRects: IntArray)

    /** Shows nothing: the frame's region is empty. */
    abstract fun clear()
}
