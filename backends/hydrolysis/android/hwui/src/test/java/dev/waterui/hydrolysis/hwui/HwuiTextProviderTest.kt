package dev.waterui.hydrolysis.hwui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The default-style row of the text wire: row zero of every packed shape
 * request is the paragraph's base paint — a zero-length run whose size,
 * weight, family and line height the layout's `TextPaint` defaults follow.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class HwuiTextProviderTest {
    private fun provider() = HwuiTextProvider()

    private fun run(
        start: Int,
        end: Int,
        size: Float,
        weight: Int,
        lineHeight: Float?,
    ): IntArray {
        val run = IntArray(TextWire.SPAN_WORDS)
        run[TextWire.START] = start
        run[TextWire.END] = end
        run[TextWire.FAMILY] = TextWire.DEFAULT_FAMILY
        run[TextWire.SIZE] = java.lang.Float.floatToRawIntBits(size)
        run[TextWire.WEIGHT] = weight
        if (lineHeight != null) {
            run[TextWire.FLAGS] = TextWire.HAS_LINE_HEIGHT
            run[TextWire.LINE_HEIGHT] = java.lang.Float.floatToRawIntBits(lineHeight)
        }
        return run
    }

    private fun lines(reply: FloatArray): Int =
        (reply.size - TextWire.REPLY_HEADER) / TextWire.LINE_WORDS

    private fun shape(
        provider: HwuiTextProvider,
        id: Int,
        text: String,
        runs: List<IntArray>,
    ): FloatArray {
        val spans = IntArray(runs.sumOf { it.size })
        var at = 0
        for (run in runs) {
            run.copyInto(spans, at)
            at += run.size
        }
        return provider.shape(
            id,
            text,
            runs.size,
            spans,
            emptyArray(),
            "",
            TextWire.UNBOUNDED,
            TextWire.NO_LINE_LIMIT,
            0,
        )
    }

    @Test
    fun `a discarded display list is re-recorded from the retained layout`() {
        val provider = provider()
        shape(provider, 100, "Cached text", listOf(run(0, 0, 16f, 400, null)))
        val node = provider.node(100)
        assertTrue(node.hasDisplayList())
        // Model the missing display list observed when returning to a
        // cached text value (#3739).
        node.discardDisplayList()
        assertFalse(node.hasDisplayList())
        // The same retained node returns with its recording restored —
        // no reshape, no new id, no substitute node.
        val restored = provider.node(100)
        assertSame(node, restored)
        assertTrue(restored.hasDisplayList())
        assertSame(restored, provider.node(100))
        provider.release(100)
    }

    @Test
    fun `the baseline row drives an empty text's line height`() {
        val provider = provider()
        // Baseline: 32px, weight 400, an explicit 40px line height, no family.
        val reply = shape(provider, 1, "", listOf(run(0, 0, 32f, 400, 40f)))
        assertEquals("an empty text lays out one baseline line", 1, lines(reply))
        assertEquals("the baseline line is its explicit height", 40f, reply[1], 0.001f)
        provider.release(1)
    }

    @Test
    fun `a styled run agrees with the same baseline on empty text`() {
        val provider = provider()
        val empty = shape(provider, 2, "", listOf(run(0, 0, 32f, 400, 40f)))
        val filled =
            shape(
                provider,
                3,
                "M",
                listOf(run(0, 0, 32f, 400, 40f), run(0, 1, 32f, 400, 40f)),
            )
        assertEquals(lines(empty), lines(filled))
        assertEquals(
            "the baseline and the run's explicit height agree",
            empty[1],
            filled[1],
            1f,
        )
        assertEquals(
            "the first baseline agrees",
            empty[TextWire.REPLY_HEADER + 2],
            filled[TextWire.REPLY_HEADER + 2],
            1f,
        )
        provider.release(2)
        provider.release(3)
    }

    @Test
    fun `a trailing newline line takes the baseline line height`() {
        val provider = provider()
        // "M\n" — the second line is an empty trailing paragraph, which no
        // span covers; its height comes from the paint's virtual metrics.
        val reply =
            shape(
                provider,
                6,
                "M\n",
                listOf(run(0, 0, 32f, 400, 40f), run(0, 1, 32f, 400, 40f)),
            )
        assertEquals(2, lines(reply))
        assertEquals(80f, reply[1], 0.001f)
        val line = { l: Int -> TextWire.REPLY_HEADER + l * TextWire.LINE_WORDS }
        assertEquals(40f, reply[line(0) + 1], 0.001f)
        assertEquals(40f, reply[line(1) + 1], 0.001f)
        assertEquals(40f, reply[line(1) + 2] - reply[line(0) + 2], 0.001f)
        provider.release(6)
    }

    @Test
    fun `the baseline paint's own size applies when no run overrides it`() {
        val provider = provider()
        val small = shape(provider, 4, "", listOf(run(0, 0, 16f, 400, null)))
        val large = shape(provider, 5, "", listOf(run(0, 0, 32f, 400, null)))
        assertTrue(
            "a 32px baseline is taller than a 16px one (${large[1]} vs ${small[1]})",
            large[1] > small[1],
        )
        provider.release(4)
        provider.release(5)
    }
    @Test
    fun `a bidirectional selection returns one rectangle per disjoint run`() {
        val provider = provider()
        val reply =
            shape(
                provider,
                7,
                "abc \u05d0\u05d1\u05d2 def",
                listOf(run(0, 0, 20f, 400, null), run(0, 11, 20f, 400, null)),
            )
        val rects = provider.selectionRects(7, 2, 5)
        assertEquals(
            "Latin 'c ' and Hebrew 'alef' are disjoint runs",
            8,
            rects.size,
        )
        // The caret midpoint between offsets 5 and 6 sits inside the
        // unselected Hebrew bet — no selection rectangle may cover it.
        val betX =
            (provider.caretRect(7, 5, false)[0] + provider.caretRect(7, 6, false)[0]) / 2f
        val y = (rects[1] + rects[3]) / 2f
        for (i in rects.indices step 4) {
            assertFalse(
                "the gap between selected runs draws nothing",
                betX >= rects[i] && betX <= rects[i + 2] &&
                    y >= rects[i + 1] && y <= rects[i + 3],
            )
        }
        assertEquals(
            "an empty selection has no rectangles",
            0,
            provider.selectionRects(7, 3, 3).size,
        )
        provider.release(7)
    }
}
