package dev.waterui.hydrolysis

import android.content.Context
import android.graphics.ColorSpace
import android.view.View

/**
 * A band the [PlatformViewRegistry] stacks right after an ordered platform
 * view: an ordinary [View] showing the self-drawn content a frame paints
 * above that view and beneath the next one (water-rs/waterui#2225). Being a
 * View in paint order, it composites between the platform views the way the
 * retained tree orders them.
 *
 * The painter supplies the realization through
 * [PlatformViewRegistry.bandFactory]; the session's [show] and [clear]
 * calls tell it a frame's image is complete. Both are called on the UI
 * thread ahead of the next traversal, so the band draws the change in the
 * same view frame as every other band the frame changed.
 */
abstract class RegionBand(context: Context) : View(context) {
    /**
     * Shows the newest image the band's producer delivered, its pixels
     * encoded in [colorSpace] — the colour space the producer rendered it in.
     */
    abstract fun show(colorSpace: ColorSpace)

    /** Shows nothing: the frame's region is empty. */
    abstract fun clear()
}
