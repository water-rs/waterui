package dev.waterui.hydrolysis.hwui

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

/** The stretch stage's wire values map onto the shared AGSL's uniforms. */
class StretchStageUniformsTest {
    @Test
    fun theStageValuesMapOntoTheShadersUniforms() {
        // A second stage: the mapping reads from its own base.
        val values = FloatArray(24) { -1f }
        floatArrayOf(2f, 0f, 0f, 2f, 20f, 50f, 100f, 200f, 0f, -0.02f, 0.7f, 0f)
            .copyInto(values, 12)
        val uniforms = StretchStageUniforms(values, 12)
        assertArrayEquals(floatArrayOf(0f, 0f), uniforms.origin, 0f)
        assertArrayEquals(floatArrayOf(2f, 0f, 0f, 2f), uniforms.frameLinear, 0f)
        assertArrayEquals(floatArrayOf(20f, 50f), uniforms.frameOffset, 0f)
        assertArrayEquals(floatArrayOf(100f, 200f), uniforms.size, 0f)
        assertArrayEquals(floatArrayOf(0f, -0.02f), uniforms.direction, 0f)
        assertEquals(0.7f, uniforms.strength, 0f)
    }
}
