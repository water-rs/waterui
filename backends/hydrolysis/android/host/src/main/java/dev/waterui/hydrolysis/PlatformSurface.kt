package dev.waterui.hydrolysis

/**
 * A platform view that only *draws*: a surface the session renders into, such
 * as the `TextureView` of a GPU content view under the HWUI painter. The
 * registry mounts it like any platform view — placed, clipped and ordered by
 * the retained tree — but input over it stays with the retained tree, which
 * routes it to the content the surface shows.
 */
interface PlatformSurface {
    /**
     * The registry mounted this view for the placement [id] of [session]; the
     * session addresses the view's surface by that id.
     */
    fun placed(session: HydrolysisSession, id: Long)
}
