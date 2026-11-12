package dev.waterui.hydrolysis

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.widget.FrameLayout
import java.util.concurrent.CountDownLatch

/**
 * The Java host of the GPU video player's decoder plane
 * (water-rs/waterui#1812): the object `waterui-video-gpu` calls to obtain a
 * `Surface` the platform decoder renders into directly. Protected content,
 * A/V tunneling and audio offload need it; the default playback never asks.
 *
 * The view is a platform view mounted at the player's frame, below the
 * picture the GPU draws. It holds no `SurfaceView` until the player acquires
 * a `Surface`: a secure `SurfaceView` must be secured before it joins the
 * window, so the plane builds one per acquisition with the flag the player
 * asks for, and removes it again on deactivation.
 *
 * `acquireVideoSurface` and `deactivateVideoSurface` are the contract's
 * entry points; they run on the player's decoder thread.
 */
internal class VideoSurfaceHostView(context: Context) : FrameLayout(context), PlatformSurface {
    private val main = Handler(Looper.getMainLooper())
    private var session: HydrolysisSession? = null
    private var id = 0L
    private var plane: SurfaceView? = null
    private var created: CountDownLatch? = null
    private var disposed = false

    init {
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    override fun placed(session: HydrolysisSession, id: Long) {
        check(this.session == null) { "hydrolysis: a video plane was placed twice" }
        this.session = session
        this.id = id
        session.videoPlaneAttached(id, this)
    }

    /**
     * Builds a plane `SurfaceView` (secured when [secure]) and returns its
     * `Surface` once the platform created it. Blocks the calling decoder
     * thread on the creation callback; never call it on the main thread.
     */
    @Suppress("unused") // called from native code through the host contract
    fun acquireVideoSurface(secure: Boolean): Surface {
        check(Looper.myLooper() != Looper.getMainLooper()) {
            "hydrolysis: acquireVideoSurface blocks and must not run on the main thread"
        }
        val latch = CountDownLatch(1)
        var acquired: Surface? = null
        main.post {
            if (disposed) {
                latch.countDown()
                return@post
            }
            removePlane()
            val view = SurfaceView(context)
            // Secure surfaces must be secured before the window attachment.
            view.setSecure(secure)
            view.holder.addCallback(
                object : SurfaceHolder.Callback {
                    override fun surfaceCreated(holder: SurfaceHolder) {
                        acquired = holder.surface
                        latch.countDown()
                    }

                    override fun surfaceChanged(
                        holder: SurfaceHolder,
                        format: Int,
                        width: Int,
                        height: Int,
                    ) = Unit

                    override fun surfaceDestroyed(holder: SurfaceHolder) {
                        session?.videoPlaneSurfaceDestroyed(id)
                    }
                },
            )
            plane = view
            created = latch
            addView(view, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        }
        latch.await()
        return checkNotNull(acquired) { "hydrolysis: the video plane was disposed before its Surface existed" }
    }

    /** The player released its `Surface`; the plane's view goes with it. */
    @Suppress("unused") // called from native code through the host contract
    fun deactivateVideoSurface() {
        main.post { removePlane() }
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        disposed = false
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        // A host view that leaves the window never gets its `Surface`: wake
        // an acquisition waiting on it so the decoder fails instead of hanging.
        disposed = true
        created?.countDown()
    }

    private fun removePlane() {
        plane?.let { removeView(it) }
        plane = null
        created = null
    }

    companion object {
        /**
         * The platform-view kind the native session names in the placement
         * of a decoder plane — the key the registry registers it under.
         */
        const val KIND = "dev.waterui.hydrolysis.video-surface-host"
    }
}
