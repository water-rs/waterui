package dev.waterui.hydrolysis.hwui

import android.graphics.Canvas
import android.graphics.Matrix
import android.util.Log
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.OrderedContent
import java.nio.ByteBuffer

/**
 * The HWUI painter: replays the session's per-frame command buffer into
 * retained [android.graphics.RenderNode]s and draws the top-level node runs
 * in the host's `dispatchDraw`, interleaved with platform views in session
 * order. Unlike the GPU painter it mounts no band; RenderThread composites.
 */
class HwuiPainter(private val host: HydrolysisHostView) : OrderedContent {
    val text: HwuiTextProvider = HwuiTextProvider()
    val sink: RenderNodeSink = RenderNodeSink(text)
    private val decoder = CommandDecoder(sink)
    private var clearColor = 0L
    private var hasClear = false
    private val inverse = Matrix()
    private val point = FloatArray(2)

    fun attach() {
        host.setOrderedContent(this)
    }

    fun detach() {
        host.setOrderedContent(null)
        sink.clear()
    }

    /**
     * Replays the first `length` bytes of the session's command buffer, on
     * the UI thread. `clear` is the window background as a `ColorLong` when
     * `hasClear`.
     *
     * @throws CommandBufferException when the frame is malformed; the
     *   session must not continue past a frame the painter refused.
     */
    fun replay(buffer: ByteBuffer, length: Int, clear: Long, hasClear: Boolean) {
        try {
            decoder.decode(buffer, length)
        } catch (error: CommandBufferException) {
            Log.e(TAG, "refused HWUI frame after sequence ${decoder.lastSequence}", error)
            throw error
        }
        clearColor = clear
        this.hasClear = hasClear
        host.invalidate()
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
