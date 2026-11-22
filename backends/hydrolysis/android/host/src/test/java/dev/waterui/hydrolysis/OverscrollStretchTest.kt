package dev.waterui.hydrolysis

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The geometry a platform view's overscroll stretch needs from the
 * placement the engine publishes (water-rs/waterui#2225): the stretch
 * filter's parameters carried to the View's pixels, and the region the
 * warped View draws into.
 */
class OverscrollStretchTest {
    private val params = floatArrayOf(1f, 0f, 0f, 1f, 10f, 20f, 100f, 200f, 0f, -0.5f)

    /** The frame is the one thing density scales: it maps to window pixels. */
    @Test
    fun uniformsCarryTheFrameToPixelsAndKeepTheViewportAndDirection() {
        val uniforms = StretchUniforms(OverscrollStretch(params, 0.7f), 2.5f, 25, 50)
        assertArrayEquals(floatArrayOf(25f, 50f), uniforms.origin, 0f)
        assertArrayEquals(floatArrayOf(2.5f, 0f, 0f, 2.5f), uniforms.frameLinear, 0f)
        assertArrayEquals(floatArrayOf(25f, 50f), uniforms.frameOffset, 0f)
        assertArrayEquals(floatArrayOf(100f, 200f), uniforms.size, 0f)
        assertArrayEquals(floatArrayOf(0f, -0.5f), uniforms.direction, 0f)
        assertEquals(0.7f, uniforms.strength, 0f)
    }

    /** A malformed parameter list fails loudly instead of warping with garbage. */
    @Test(expected = IllegalArgumentException::class)
    fun aStretchCarriesExactlyTheFilterParameters() {
        OverscrollStretch(floatArrayOf(1f, 0f, 0f, 1f), 0.7f)
    }

    /**
     * A view filling the lower part of the viewport stretches past its
     * frame: the region is the viewport, and the mounted view keeps its
     * frame's size at the same window position inside it.
     */
    @Test
    fun theRegionEnclosesTheFrameAndTheMountedViewKeepsItsPlace() {
        val layout =
            StretchLayout.of(
                frame = floatArrayOf(10f, 120f, 300f, 160f),
                region = floatArrayOf(0f, 100f, 320f, 400f),
                density = 2f,
            )
        assertEquals(0, layout.regionLeft)
        assertEquals(200, layout.regionTop)
        assertEquals(640, layout.regionWidth)
        assertEquals(800, layout.regionHeight)
        assertEquals(20, layout.mountedLeft)
        assertEquals(40, layout.mountedTop)
        assertEquals(600, layout.mountedWidth)
        assertEquals(320, layout.mountedHeight)
    }

    /** Pixel truncation never cuts the frame's last pixel off the region. */
    @Test
    fun theRegionRoundsOutwardPastTheFrame() {
        val layout =
            StretchLayout.of(
                frame = floatArrayOf(0.4f, 0.4f, 10.3f, 10.3f),
                region = floatArrayOf(0.4f, 0.4f, 10.3f, 10.3f),
                density = 2.625f,
            )
        val frameRight = (0.4f * 2.625f).toInt() + (10.3f * 2.625f).toInt()
        val frameBottom = (0.4f * 2.625f).toInt() + (10.3f * 2.625f).toInt()
        assertTrue(layout.regionLeft + layout.regionWidth >= frameRight)
        assertTrue(layout.regionTop + layout.regionHeight >= frameBottom)
        assertTrue(layout.regionLeft <= (0.4f * 2.625f).toInt())
        assertEquals(layout.mountedLeft, (0.4f * 2.625f).toInt() - layout.regionLeft)
    }
}
