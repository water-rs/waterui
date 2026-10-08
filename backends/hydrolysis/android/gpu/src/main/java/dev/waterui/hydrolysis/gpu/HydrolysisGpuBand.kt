package dev.waterui.hydrolysis.gpu

import android.annotation.SuppressLint
import android.content.Context
import android.util.AttributeSet
import android.view.Choreographer
import android.view.SurfaceHolder
import android.view.SurfaceView
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.NativeBridge
import java.util.concurrent.TimeUnit

/**
 * The GPU attachment: a [SurfaceView] band whose `SurfaceHolder`
 * lifecycle owns the native presentation attachment.
 *
 * The band sits as child 0 of the [dev.waterui.hydrolysis.HydrolysisHostView]
 * so every host child (platform-view overlay, native embeddings) draws above
 * it. It is the only Kotlin object that touches a presentation `Surface` —
 * the host view stays painter-independent.
 *
 * Every `surfaceCreated` bumps [generation]; attach/change/destroy carry that
 * number across JNI so a stale `surfaceDestroyed` can never tear down the
 * surface a newer `surfaceCreated` produced. The native side owns the
 * `ANativeWindow` lease and destroys its wgpu surface before the band's
 * surface is released — SurfaceFlinger ordering is preserved.
 */
// Programmatic-only: the band cannot exist without a live session, so it is
// never inflated by a layout tool.
@SuppressLint("ViewConstructor")
class HydrolysisGpuBand
@JvmOverloads
constructor(
    context: Context,
    private val session: HydrolysisSession,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : SurfaceView(context, attrs, defStyleAttr), SurfaceHolder.Callback {

    /**
     * Attach retries are bounded by one second of vsync frames: SurfaceView's
     * BLAST placeholder keeps the window's producer connection busy for the
     * first traversal, and `eglCreateWindowSurface` fails "already connected"
     * until SurfaceFlinger releases it. Retrying once per frame is not idle
     * pumping — the callback list is empty the moment attach lands or the
     * deadline passes.
     */
    private var attachDeadlineNanos = 0L
    private var attachPending = false

    /** The host's attachment-generation counter, owned on the UI thread. */
    private var generation = 0L

    private val attachRetry = Choreographer.FrameCallback {
        attachPending = false
        attemptAttach()
    }

    init {
        holder.addCallback(this)
        // The band draws only while its surface exists; an idle engine never
        // requests frames, so a parked band costs nothing.
        setZOrderOnTop(false)
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        generation += 1
        attachDeadlineNanos = System.nanoTime() + ATTACH_RETRY_NANOS
        attemptAttach()
    }

    private fun attemptAttach() {
        val surface = holder.surface
        if (surface == null || !surface.isValid) return
        // Outside the retry: a destroyed session is a named error, not an
        // attach failure to retry.
        val sessionPtr = session.nativePtr(NativeBridge::nativeSurfaceAttached.name)
        val attached =
            try {
                NativeBridge.nativeSurfaceAttached(
                    sessionPtr,
                    surface,
                    width,
                    height,
                    generation,
                )
            } catch (error: RuntimeException) {
                retryAttachOrThrow(error)
                return
            }
        if (!attached) {
            retryAttachOrThrow(
                IllegalStateException(
                    "hydrolysis android: GPU surface attach failed (generation $generation)",
                ),
            )
        }
    }

    private fun retryAttachOrThrow(error: RuntimeException) {
        if (System.nanoTime() < attachDeadlineNanos) {
            if (!attachPending) {
                attachPending = true
                Choreographer.getInstance().postFrameCallback(attachRetry)
            }
        } else {
            throw error
        }
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        NativeBridge.nativeSurfaceChanged(
            session.nativePtr(NativeBridge::nativeSurfaceChanged.name),
            width,
            height,
            generation,
        )
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        attachPending = false
        Choreographer.getInstance().removeFrameCallback(attachRetry)
        NativeBridge.nativeSurfaceDestroyed(
            session.nativePtr(NativeBridge::nativeSurfaceDestroyed.name),
            generation,
        )
    }

    private companion object {
        val ATTACH_RETRY_NANOS: Long = TimeUnit.SECONDS.toNanos(1)
    }
}
