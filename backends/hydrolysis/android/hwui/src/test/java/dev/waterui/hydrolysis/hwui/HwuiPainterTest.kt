package dev.waterui.hydrolysis.hwui

import android.app.Activity
import android.graphics.Color
import android.graphics.RenderNode
import dev.waterui.hydrolysis.HydrolysisHostView
import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.Shadows.shadowOf

/** The session-owned painter's lifecycle: attach/detach/destroy and what
 * each does to the retained nodes. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class HwuiPainterTest {
    private class RecordingHost(activity: Activity) : HydrolysisHostView(activity, null)

    private fun host(): RecordingHost =
        RecordingHost(Robolectric.buildActivity(Activity::class.java).get())

    private fun nodeWithDisplayList(sink: RenderNodeSink, id: Int): RenderNode {
        sink.createNode(id)
        val node = sink.node(id)
        node.setPosition(0, 0, 10, 10)
        node.beginRecording(10, 10).drawColor(Color.RED)
        node.endRecording()
        return node
    }

    private fun shapeText(painter: HwuiPainter, id: Int) {
        val spans =
            intArrayOf(
                0, 0, TextWire.DEFAULT_FAMILY,
                16f.toRawBits(), 400, 0, 0, 0,
                0, 0, 0, 0,
            )
        painter.text.shape(id, "", 1, spans, emptyArray(), "", TextWire.UNBOUNDED, TextWire.NO_LINE_LIMIT, 0)
    }

    private fun frame(sequence: Int): ByteBuffer =
        ByteBuffer.allocateDirect(Protocol.HEADER_BYTES)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(Protocol.MAGIC)
            .putInt(sequence)
            .putInt(Protocol.HEADER_BYTES)
            .apply { rewind() }

    @Test
    fun `detach keeps the retained nodes and a rebind replays them`() {
        val painter = HwuiPainter()
        val node = nodeWithDisplayList(painter.sink, 1)
        shapeText(painter, 1)
        val textNode = painter.text.node(1)

        val old = host()
        painter.attach(old)
        painter.detach(old)
        // Detach must not clear retained data.
        assertSame(node, painter.sink.node(1))
        assertSame(textNode, painter.text.node(1))
        assertTrue(painter.sink.node(1).hasDisplayList())

        val fresh = host()
        painter.attach(fresh)
        assertSame(node, painter.sink.node(1))
        assertSame(textNode, painter.text.node(1))
        // A stale host's detach must not unbind the new host.
        painter.detach(old)
        assertSame(node, painter.sink.node(1))

        // The replay invalidates the container that draws the content —
        // the host's own invalidation would not re-record its cached list.
        val oldContainer = old.getChildAt(0)
        val freshContainer = fresh.getChildAt(0)
        shadowOf(oldContainer).clearWasInvalidated()
        shadowOf(freshContainer).clearWasInvalidated()
        painter.replay(frame(1), Protocol.HEADER_BYTES, 0L, false)
        assertFalse(shadowOf(oldContainer).wasInvalidated())
        assertTrue(shadowOf(freshContainer).wasInvalidated())

        painter.detach(fresh)
        painter.destroy()
        assertThrows(CommandBufferException::class.java) { painter.sink.node(1) }
        assertThrows(CommandBufferException::class.java) { painter.text.node(1) }
    }

    @Test
    fun `a platform-view binding records the native view inside its portal`() {
        val painter = HwuiPainter()
        painter.sink.createNode(5)
        painter.sink.bindPlatformView(
            5, 42, 0f, 0f, 15f, 10f, 2f,
            floatArrayOf(1f, 0f, 0f, 0f, 1f, 0f, 0f, 0f, 1f),
        )
        var seen: Triple<Long, Int, Int>? = null
        painter.preparePlatformViews { id, canvas, width, height, _ ->
            seen = Triple(id, width, height)
            canvas.drawColor(Color.RED)
        }
        val (id, width, height) = seen ?: error("the binding never reached the recorder")
        assertEquals(42L, id)
        // 15x10 logical at density 2 is 30x20 physical pixels.
        assertEquals(30, width)
        assertEquals(20, height)
        assertTrue(painter.sink.node(5).hasDisplayList())
    }

    @Test
    fun `a bound painter rejects a second host and destroy ordering`() {
        val painter = HwuiPainter()
        val first = host()
        val second = host()
        painter.attach(first)
        assertThrows(IllegalStateException::class.java) { painter.attach(second) }
        assertThrows(IllegalStateException::class.java) { painter.destroy() }
        // The refused destroy left the painter usable.
        painter.detach(first)
        painter.destroy()
        assertThrows(IllegalStateException::class.java) { painter.attach(host()) }
        assertThrows(IllegalStateException::class.java) {
            painter.replay(frame(1), Protocol.HEADER_BYTES, 0L, false)
        }
        assertThrows(IllegalStateException::class.java) { painter.destroy() }
    }
}
