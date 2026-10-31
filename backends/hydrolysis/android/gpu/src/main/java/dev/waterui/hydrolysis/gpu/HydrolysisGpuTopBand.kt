package dev.waterui.hydrolysis.gpu

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.PixelFormat
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.NativeBridge

/**
 * The GPU top band: a [SurfaceView] installed above the platform-view
 * overlay, carrying the self-drawn entries a frame paints above its
 * embedded platform views — a popover, sheet or menu over a web page
 * (water-rs/waterui#2225).
 *
 * The base [HydrolysisGpuBand] sits beneath the overlay as child 0; this
 * band goes in as the host's last child so its surface composites above
 * every embedded view. The renderer publishes into it only the frame's
 * entries above the topmost platform view's paint order; when a frame has
 * none, the surface stays transparent.
 *
 * The band never takes a touch: the retained tree's ordered hit test
 * decides gesture ownership among content and embedded views, and a pixel
 * an interactive entry above a platform view does not cover still belongs
 * to whatever lies beneath. `dispatchTouchEvent` returning `false` hands
 * every touch back to the container below.
 *
 * Every `surfaceCreated` bumps [generation]; attach/change/destroy carry
 * that number across JNI so a stale `surfaceDestroyed` can never tear
 * down the surface a newer `surfaceCreated` produced — the same contract
 * the base band keeps with `nativeSurface*`.
 */
// Programmatic-only: the band cannot exist without a live session, so it is
// never inflated by a layout tool.
@SuppressLint("ViewConstructor")
class HydrolysisGpuTopBand
@JvmOverloads
constructor(
    context: Context,
    private val session: HydrolysisSession,
) : SurfaceView(context), SurfaceHolder.Callback {

    /** The host's attachment-generation counter, owned on the UI thread. */
    private var generation = 0L

    init {
        holder.addCallback(this)
        // Above-band content is mostly transparency — the surface needs an
        // alpha-capable format or the clear would black out the window.
        holder.setFormat(PixelFormat.TRANSLUCENT)
        // Touches belong to the entries the retained tree covers — the
        // ordered hit test routes them; the band itself answers none.
        isClickable = false
        isLongClickable = false
        isFocusable = false
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean = false

    override fun surfaceCreated(holder: SurfaceHolder) {
        generation += 1
        session.withNativePtr(NativeBridge::nativeTopSurfaceAttached.name) { ptr ->
            check(
                NativeBridge.nativeTopSurfaceAttached(
                    ptr,
                    holder.surface,
                    width,
                    height,
                    generation,
                ),
            ) { "hydrolysis android: GPU top surface attach failed (generation $generation)" }
        }
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        session.withNativePtr(NativeBridge::nativeTopSurfaceChanged.name) { ptr ->
            NativeBridge.nativeTopSurfaceChanged(ptr, width, height, generation)
        }
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        session.withNativePtr(NativeBridge::nativeTopSurfaceDestroyed.name) { ptr ->
            NativeBridge.nativeTopSurfaceDestroyed(ptr, generation)
        }
    }
}
