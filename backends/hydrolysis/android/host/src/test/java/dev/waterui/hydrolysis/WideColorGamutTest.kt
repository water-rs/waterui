package dev.waterui.hydrolysis

import android.content.pm.ActivityInfo
import android.view.Display
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.annotation.internal.DoNotInstrument
import org.robolectric.shadows.ShadowDisplay

@Implements(Display::class)
class ShadowWideColorDisplay : ShadowDisplay() {
    companion object {
        var wideColorGamut = false
    }

    @Implementation
    fun isWideColorGamut(): Boolean = wideColorGamut
}

@RunWith(RobolectricTestRunner::class)
@Config(
    shadows = [ShadowNativeBridge::class, ShadowWideColorDisplay::class],
    instrumentedPackages = ["dev.waterui.hydrolysis"],
)
@DoNotInstrument
class WideColorGamutTest {
    private val worker = CaptiveSyncWorker()

    @Before
    fun reset() {
        ShadowNativeBridge.reset()
        HydrolysisEnvironment.resetForTest()
        HydrolysisEnvironment.syncExecutor = worker
        ShadowWideColorDisplay.wideColorGamut = false
    }

    @Test
    fun requestsWideColorGamutWhenAttachedDisplaySupportsIt() {
        ShadowWideColorDisplay.wideColorGamut = true
        val activity = Robolectric.buildActivity(RecordingHydrolysisActivity::class.java).setup().get()
        assertEquals(ActivityInfo.COLOR_MODE_WIDE_COLOR_GAMUT, activity.window.colorMode)
    }

    @Test
    fun leavesDefaultColorModeWhenAttachedDisplayIsNotWideColorGamut() {
        ShadowWideColorDisplay.wideColorGamut = false
        val activity = Robolectric.buildActivity(RecordingHydrolysisActivity::class.java).setup().get()
        assertEquals(ActivityInfo.COLOR_MODE_DEFAULT, activity.window.colorMode)
    }
}
