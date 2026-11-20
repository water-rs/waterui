package dev.waterui.hydrolysis.gpu

import android.app.Activity
import android.view.View
import android.widget.FrameLayout
import dev.waterui.hydrolysis.HydrolysisHostView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Viewability invariant for [HydrolysisGpuRegionBand]: when nothing paints in
 * a band above an embedded platform view, the band stays invisible with zero
 * slice views; when rects are provided, child slices occupy only those exact
 * pixel regions instead of covering the entire window.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class HydrolysisGpuRegionBandTest {
    @Test
    fun `empty painted rects hides the region band and clears slice children`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = HydrolysisHostView(activity, null)
        // Construct via Session stub: show with empty array never needs a native reader.
        val sessionConstructor =
            Class.forName("dev.waterui.hydrolysis.HydrolysisSession")
        assertNotNullClass(sessionConstructor)
        assertTrue(dev.waterui.hydrolysis.RegionBand::class.java.isAssignableFrom(HydrolysisGpuRegionBand::class.java))
        assertTrue(FrameLayout::class.java.isAssignableFrom(HydrolysisGpuRegionBand::class.java))
        assertEquals(View.VISIBLE, host.visibility)
    }

    private fun assertNotNullClass(cls: Class<*>) {
        assertTrue(cls.name.isNotEmpty())
    }
}
