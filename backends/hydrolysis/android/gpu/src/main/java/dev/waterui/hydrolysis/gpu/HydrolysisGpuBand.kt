package dev.waterui.hydrolysis.gpu

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.PixelFormat
import android.os.Build
import android.util.AttributeSet
import android.view.Display
import android.view.SurfaceHolder
import android.view.SurfaceView
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.NativeBridge

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
 * `ANativeWindow` lease and destroys its compositor target before the band's
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

    /** The host's attachment-generation counter, owned on the UI thread. */
    private var generation = 0L

    init {
        holder.addCallback(this)
        // The band draws only while its surface exists; an idle engine never
        // requests frames, so a parked band costs nothing.
        setZOrderOnTop(false)
        // A translucent window asks the surface for an alpha-capable format —
        // the engine's own compositor planes carry the alpha either way, but
        // an opaque parent format keeps the blend from reaching the window
        // behind it. `setFormat` recreates the surface, so the native attach
        // generation machinery re-leases the new `ANativeWindow` itself.
        session.addTranslucencyListener { (translucent, _) ->
            holder.setFormat(if (translucent) PixelFormat.RGBA_8888 else PixelFormat.OPAQUE)
        }
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        generation += 1
        val peakRefreshHz = peakRefreshHz()
        val attachedDisplay = checkNotNull(display)
        val (normalRefreshHz, highRefreshHz) =
            if (Build.VERSION.SDK_INT >= 36 && attachedDisplay.hasArrSupport()) {
                val rates = attachedDisplay.supportedRefreshRates
                val suggestions =
                    listOf(Display.FRAME_RATE_CATEGORY_NORMAL, Display.FRAME_RATE_CATEGORY_HIGH).map {
                        category ->
                        val suggested =
                            attachedDisplay.getSuggestedFrameRate(category).coerceAtMost(peakRefreshHz)
                        // Category suggestions need not be supported rates: a 90 Hz
                        // vote on Pixel can resolve to 80 Hz. Round upward so the
                        // chosen display cadence does not undershoot the suggestion.
                        rates.filter { it >= suggested }.min()
                    }
                suggestions[0] to suggestions[1]
            } else {
                peakRefreshHz to peakRefreshHz
            }
        session.withNativePtr(NativeBridge::nativeSurfaceAttached.name) { ptr ->
            check(
                NativeBridge.nativeSurfaceAttached(
                    ptr,
                    holder.surface,
                    peakRefreshHz,
                    normalRefreshHz,
                    highRefreshHz,
                    width,
                    height,
                    generation,
                ),
            ) { "hydrolysis android: GPU surface attach failed (generation $generation)" }
        }
    }

    /**
     * The highest refresh rate the band's display offers at its current
     * resolution — the rate a high-refresh demand asks for. A band whose
     * surface exists is attached to a window, so it has a display.
     */
    private fun peakRefreshHz(): Float {
        val display =
            checkNotNull(display) { "hydrolysis android: GPU band surface exists without a display" }
        val current = display.mode
        return display.supportedModes
            .filter {
                it.physicalWidth == current.physicalWidth &&
                    it.physicalHeight == current.physicalHeight
            }
            .maxOf { it.refreshRate }
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        session.withNativePtr(NativeBridge::nativeSurfaceChanged.name) { ptr ->
            NativeBridge.nativeSurfaceChanged(ptr, width, height, generation)
        }
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        session.withNativePtr(NativeBridge::nativeSurfaceDestroyed.name) { ptr ->
            NativeBridge.nativeSurfaceDestroyed(ptr, generation)
        }
    }
}
