package dev.waterui.hydrolysis.gpu

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.ColorSpace
import android.graphics.PixelFormat
import android.hardware.HardwareBuffer
import android.media.Image
import android.media.ImageReader
import android.view.MotionEvent
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.NativeBridge
import dev.waterui.hydrolysis.RegionBand

/**
 * The GPU painter's band above one embedded platform view: the self-drawn
 * content a frame paints above that view and beneath the next one — a
 * popover, sheet or menu over a web page (water-rs/waterui#2225).
 *
 * It is an ordinary View, not a surface: the platform-view registry stacks
 * it right after its platform view, and HWUI composites it in child order,
 * so `WebView A → popover → WebView B` draws in that order. Behind it is an
 * [ImageReader] the engine presents the region into as a plain swapchain;
 * [show] takes the newest image and [onDraw] draws its hardware buffer as
 * a bitmap, without a copy, tagged with the colour space the swapchain
 * rendered it in.
 *
 * The engine only updates a band once the GPU finished the frame's images,
 * and updates every band a frame changed in one UI-thread turn, so they all
 * draw in one view frame. A shown image stays open until the next update:
 * the view frame that drew it may still be reading it on the render thread
 * when its successor arrives.
 *
 * Each reader carries a new [generation] across JNI — on attach and on
 * every resize, readers being fixed-size — so a stale detach never clears
 * a newer reader. The band takes no touches and has no accessibility node:
 * the retained tree's ordered hit test and semantics own both.
 */
// Programmatic-only: the band cannot exist without a live session, so it is
// never inflated by a layout tool.
@SuppressLint("ViewConstructor")
class HydrolysisGpuRegionBand(
    context: Context,
    private val session: HydrolysisSession,
    /** The placement id of the platform view this band sits above. */
    private val view: Long,
) : RegionBand(context) {

    /** The band's reader-generation counter, owned on the UI thread. */
    private var generation = 0L
    private var reader: ImageReader? = null

    /** The image [bitmap] wraps. */
    private var current: Image? = null

    /** The image the previous latch showed, closed by the next one. */
    private var previous: Image? = null
    private var bitmap: Bitmap? = null

    init {
        isClickable = false
        isLongClickable = false
        isFocusable = false
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
        setWillNotDraw(false)
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean = false

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        if (width > 0 && height > 0) open(width, height)
    }

    override fun onSizeChanged(width: Int, height: Int, oldWidth: Int, oldHeight: Int) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)
        if (!isAttachedToWindow) return
        val open = reader
        if (open != null && open.width == width && open.height == height) return
        close()
        if (width > 0 && height > 0) open(width, height)
    }

    override fun onDetachedFromWindow() {
        close()
        super.onDetachedFromWindow()
    }

    private fun open(width: Int, height: Int) {
        check(reader == null) { "hydrolysis: the band above view $view opened a second reader" }
        val next =
            ImageReader.newInstance(
                width,
                height,
                PixelFormat.RGBA_8888,
                MAX_IMAGES,
                HardwareBuffer.USAGE_GPU_SAMPLED_IMAGE or HardwareBuffer.USAGE_GPU_COLOR_OUTPUT,
            )
        reader = next
        generation += 1
        session.withNativePtr(NativeBridge::nativeBandAttached.name) { ptr ->
            NativeBridge.nativeBandAttached(ptr, view, next.surface, width, height, generation)
        }
    }

    /** Detaches the engine before the reader and its images close. */
    private fun close() {
        val open = reader ?: return
        session.withNativePtr(NativeBridge::nativeBandDetached.name) { ptr ->
            NativeBridge.nativeBandDetached(ptr, view, generation)
        }
        bitmap = null
        previous?.close()
        previous = null
        current?.close()
        current = null
        open.close()
        reader = null
        invalidate()
    }

    override fun show(colorSpace: ColorSpace) {
        val open = checkNotNull(reader) { "hydrolysis: the band above view $view showed without a reader" }
        // `null` only when the frame presented into a reader this band has
        // since replaced on a resize: the new reader's first frame follows.
        val image = open.acquireLatestImage() ?: return
        val buffer = checkNotNull(image.hardwareBuffer) { "hydrolysis: band image has no buffer" }
        val wrapped =
            try {
                checkNotNull(Bitmap.wrapHardwareBuffer(buffer, colorSpace)) {
                    "hydrolysis: the band above view $view cannot wrap its RGBA_8888 buffer as $colorSpace"
                }
            } finally {
                buffer.close()
            }
        replace(image, wrapped)
    }

    override fun clear() {
        checkNotNull(reader) { "hydrolysis: the band above view $view cleared without a reader" }
        replace(null, null)
    }

    private fun replace(image: Image?, wrapped: Bitmap?) {
        previous?.close()
        previous = current
        current = image
        bitmap = wrapped
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        bitmap?.let { canvas.drawBitmap(it, 0f, 0f, null) }
    }

    private companion object {
        /**
         * The shown image, the one it replaced and still pending a view
         * frame, the one [ImageReader.acquireLatestImage] acquires, and the
         * one it probes for a newer image with.
         */
        const val MAX_IMAGES = 4
    }
}
