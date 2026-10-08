package dev.waterui.hydrolysis

import android.app.Activity
import android.view.View
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The platform-view focus contract: a child focus change inside the
 * registry container requests exactly one frame, and the value the frame
 * pulls through [PlatformViewRegistry.focusInside] reflects the change.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PlatformViewRegistryFocusTest {

    /** A host view that counts its frame requests instead of scheduling. */
    private class RecordingHostView(activity: Activity) :
        HydrolysisHostView(activity, null) {
        var frames = 0
            private set

        override fun requestFrame() {
            frames += 1
        }
    }

    @Test
    fun `a child focus change requests one frame and the pull reads it`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val hostView = RecordingHostView(activity)
        val registry = hostView.platformViewRegistry
        activity.setContentView(hostView)

        val child = View(activity)
        child.isFocusable = true
        child.isFocusableInTouchMode = true
        registry.container.addView(child)
        assertFalse(registry.focusInside())

        child.requestFocus()
        assertEquals(1, hostView.frames)
        assertTrue(registry.focusInside())

        child.clearFocus()
        assertEquals(2, hostView.frames)
        assertFalse(registry.focusInside())
    }
}
