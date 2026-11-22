package dev.waterui.hydrolysis.hwui

import android.content.Context
import android.graphics.SurfaceTexture
import android.view.Surface
import android.view.TextureView
import android.view.View
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.PlatformSurface

/**
 * The `TextureView` the HWUI painter mounts for each GPU content view
 * (water-rs/waterui#1812). A `TextureView` draws through its owner's display
 * list, so the retained tree's clip, transform, alpha and z-order apply to the
 * GPU content exactly as to any other view; a `SurfaceView` would punch a
 * window-level hole instead. The session renders into [Surface] and reports
 * its lifecycle by the placement id the registry passes to [placed].
 */
internal class HostedSurfaceView(context: Context) :
    TextureView(context), PlatformSurface, TextureView.SurfaceTextureListener {
    private var session: HydrolysisSession? = null
    private var id = 0L
    private var surface: Surface? = null

    init {
        // GPU content composites over what the retained tree drew below it.
        isOpaque = false
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        surfaceTextureListener = this
    }

    override fun placed(session: HydrolysisSession, id: Long) {
        check(this.session == null) { "hydrolysis: a hosted surface was placed twice" }
        this.session = session
        this.id = id
    }

    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) {
        val session = checkNotNull(session) { "hydrolysis: a hosted surface was never placed" }
        val surface = Surface(texture)
        this.surface = surface
        session.hostedSurfaceAttached(id, surface, width, height)
    }

    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) {
        session?.hostedSurfaceResized(id, width, height)
    }

    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
        // The session drops its swapchain first: the producer end must be
        // gone before the platform reclaims the surface texture.
        session?.hostedSurfaceDestroyed(id)
        surface?.release()
        surface = null
        return true
    }

    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit

    companion object {
        /**
         * The platform-view kind the native session names in the placement of
         * each GPU content view — the key `HostedGpu` publishes under.
         */
        const val KIND = "dev.waterui.hydrolysis.hosted-gpu-surface"
    }
}
