package dev.waterui.hydrolysis.hwui

import android.graphics.Canvas
import android.os.Build
import android.graphics.Matrix
import android.util.Log
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.SessionPainter
import dev.waterui.hydrolysis.OrderedContent
import dev.waterui.hydrolysis.PlatformViewRecorder
import java.nio.ByteBuffer
import kotlin.math.ceil
import dev.waterui.hydrolysis.CalledFromNative

/**
 * The HWUI painter: replays the session's per-frame command buffer into
 * retained [android.graphics.RenderNode]s and draws the top-level node runs
 * in the host's `dispatchDraw`, interleaved with platform views in session
 * order. Unlike the GPU painter it mounts no band; RenderThread composites.
 */
class HwuiPainter : OrderedContent, SessionPainter {
    private var host: HydrolysisHostView? = null
    private var destroyed = false
    private val highRefresh = HighRefresh()

    @get:CalledFromNative
    val text: HwuiTextProvider = HwuiTextProvider()
    @get:CalledFromNative
    val sink: RenderNodeSink = RenderNodeSink(text)
    private val decoder = CommandDecoder(sink)
    private var clearColor = 0L
    private var hasClear = false
    private val inverse = Matrix()
    private val point = FloatArray(2)

    /** Binds this session-owned painter to `host`; the same painter rebinds
     * to a new host view after a configuration change, drawing the retained
     * nodes again with no Rust remount. */
    override fun attach(host: HydrolysisHostView) {
        check(!destroyed) { "hydrolysis: attaching a destroyed HWUI painter" }
        check(this.host == null || this.host === host) {
            "hydrolysis: HWUI painter is already bound to another host"
        }
        this.host = host
        host.setOrderedContent(this)
        highRefresh.attached(host)
        host.invalidate()
    }

    /** Unbinds `host`. The retained nodes, text layouts and host order are
     * the session's, not the view's — detach never clears them. */
    override fun detach(host: HydrolysisHostView) {
        if (this.host !== host) return
        highRefresh.detached(host)
        host.setOrderedContent(null)
        this.host = null
    }

    /** Final session teardown: releases every retained node and text
     * layout. A bound painter must be detached first. */
    override fun destroy() {
        check(!destroyed) { "hydrolysis: HWUI painter destroyed twice" }
        check(host == null) { "hydrolysis: detach the HWUI painter before destroying it" }
        destroyed = true
        sink.clear()
        text.clear()
    }

    override fun setHighRefresh(active: Boolean) {
        check(!destroyed) { "hydrolysis: refresh demand reached a destroyed HWUI painter" }
        highRefresh.set(active, host)
    }

    /**
     * Replays the first `length` bytes of the session's command buffer, on
     * the UI thread. `clear` is the window background as a `ColorLong` when
     * `hasClear`.
     *
     * @throws CommandBufferException when the frame is malformed; the
     *   session must not continue past a frame the painter refused.
     */
    @CalledFromNative
    fun replay(buffer: ByteBuffer, length: Int, clear: Long, hasClear: Boolean) {
        check(!destroyed) { "hydrolysis: replaying a destroyed HWUI painter" }
        try {
            decoder.decode(buffer, length)
        } catch (error: CommandBufferException) {
            Log.e(TAG, "refused HWUI frame after sequence ${decoder.lastSequence}", error)
            throw error
        }
        clearColor = clear
        this.hasClear = hasClear
        host?.invalidateContent()
    }

    override val entryCount: Int
        get() = sink.hostCount

    override fun isPlatformView(index: Int): Boolean = sink.hostKinds[index] == HostKind.PLATFORM_VIEW

    override fun platformViewId(index: Int): Long = sink.hostIds[index]

    override fun drawBackground(canvas: Canvas) {
        if (hasClear) canvas.drawColor(clearColor)
    }

    override fun drawEntry(canvas: Canvas, index: Int) {
        canvas.drawRenderNode(sink.node(nodeId(index)))
    }

    /** Whether the run's node, sized to its ink, lies under the point after its transform. */
    override fun covers(index: Int, x: Float, y: Float): Boolean {
        val node = sink.node(nodeId(index))
        if (!node.hasDisplayList()) return false
        node.getInverseMatrix(inverse)
        point[0] = x - node.left
        point[1] = y - node.top
        inverse.mapPoints(point)
        return point[0] >= 0f && point[1] >= 0f && point[0] < node.width && point[1] < node.height
    }

    /**
     * Re-records every bound platform view's portal: the binding's logical
     * bounds scale to physical pixels for the node's display list, and the
     * registry's recorder draws the mounted child's own list inside.
     */
    override fun preparePlatformViews(recorder: PlatformViewRecorder) {
        sink.preparePlatformViews { id, node, width, height, density, matrix ->
            val physicalWidth = width.toDouble() * density
            val physicalHeight = height.toDouble() * density
            if (!physicalWidth.isFinite() || !physicalHeight.isFinite() ||
                physicalWidth > Int.MAX_VALUE || physicalHeight > Int.MAX_VALUE
            ) {
                throw CommandBufferException(
                    "HWUI replay: platform view $id's extent ${width}x$height at density " +
                        "$density is past the native pixel range",
                )
            }
            val w = ceil(physicalWidth).toInt()
            val h = ceil(physicalHeight).toInt()
            node.setPosition(0, 0, w, h)
            if (w == 0 || h == 0) {
                node.discardDisplayList()
            } else {
                val canvas = node.beginRecording(w, h)
                try {
                    canvas.clipRect(0f, 0f, physicalWidth.toFloat(), physicalHeight.toFloat())
                    recorder.drawLocal(id, canvas, w, h, matrix)
                } finally {
                    node.endRecording()
                }
            }
        }
    }

    override fun platformViewAt(x: Float, y: Float): Long? {
        val host = host ?: return null
        return host.platformViewAt(x, y)
    }

    private fun nodeId(index: Int): Int {
        val id = sink.hostIds[index]
        if (id !in 0L..Int.MAX_VALUE.toLong()) {
            throw CommandBufferException("HWUI replay: host entry $index names node $id, past the dense id range")
        }
        return id.toInt()
    }

    private companion object {
        const val TAG = "HydrolysisHwui"
    }

}

/**
 * The high-refresh override for the bound host: public platform APIs only.
 * API 35+ writes `View.requestedFrameRate`; API 31–34 writes the host
 * activity window's `preferredRefreshRate`. The demand survives detach but
 * the override does not — an unbound painter restores the previous rate.
 */
private class HighRefresh {
    private var active = false
    private var appliedTo: HydrolysisHostView? = null
    private var previousFrameRate: Float? = null
    private var previousWindowRate: Float? = null

    fun set(active: Boolean, host: HydrolysisHostView?) {
        this.active = active
        if (active) {
            host?.let { apply(it) }
        } else {
            release()
        }
    }

    fun attached(host: HydrolysisHostView) {
        if (active) apply(host)
    }

    fun detached(host: HydrolysisHostView) {
        if (appliedTo === host) release()
    }

    private fun apply(host: HydrolysisHostView) {
        if (appliedTo === host) return
        release()
        if (Build.VERSION.SDK_INT >= 35) {
            previousFrameRate = host.requestedFrameRate
            host.requestedFrameRate = peakRefresh(host)
        } else {
            val window = activityWindow(host)
            previousWindowRate = window.attributes.preferredRefreshRate
            val attributes = window.attributes
            attributes.preferredRefreshRate = peakRefresh(host)
            window.attributes = attributes
        }
        appliedTo = host
    }

    private fun release() {
        val host = appliedTo ?: return
        if (Build.VERSION.SDK_INT >= 35) {
            previousFrameRate?.let { host.requestedFrameRate = it }
        } else {
            previousWindowRate?.let { rate ->
                val window = activityWindow(host)
                val attributes = window.attributes
                attributes.preferredRefreshRate = rate
                window.attributes = attributes
            }
        }
        appliedTo = null
        previousFrameRate = null
        previousWindowRate = null
    }

    private fun peakRefresh(host: HydrolysisHostView): Float {
        val display = host.display
            ?: throw IllegalStateException("hydrolysis: high refresh needs a display on the bound host")
        val current = display.mode
        return display.supportedModes
            .filter { it.physicalWidth == current.physicalWidth && it.physicalHeight == current.physicalHeight }
            .maxOf { it.refreshRate }
    }

    private fun activityWindow(host: HydrolysisHostView): android.view.Window {
        var context = host.context
        while (context is android.content.ContextWrapper && context !is android.app.Activity) {
            context = context.baseContext
        }
        val activity = context as? android.app.Activity
            ?: throw IllegalStateException("hydrolysis: high refresh on API < 35 requires a window host")
        return activity.window
    }
}
