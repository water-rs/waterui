package dev.waterui.hydrolysis

import android.content.Context
import android.view.View
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.widget.FrameLayout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf

@RunWith(RobolectricTestRunner::class)
class AccessibilityTreeEventTest {
    @Test
    fun initialSubtreeChangeIdentifiesTheHostToInvalidate() {
        val context: Context = RuntimeEnvironment.getApplication()
        shadowOf(context.getSystemService(AccessibilityManager::class.java)).setEnabled(true)
        val host = HydrolysisHostView(context)
        var dispatched = false
        val parent = object : FrameLayout(context) {
            override fun requestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
                dispatched = true
                assertSame(host, shadowOf(event).sourceRoot)
                assertEquals(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED, event.eventType)
                assertEquals(AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE, event.contentChangeTypes)
                return true
            }
        }
        parent.addView(host)
        (host.accessibilityNodeProvider as HydrolysisAccessibilityProvider).notifyTreeChanged(
            """{"events":[{"id":-1,"type":2048,"mask":1}]}""",
        )
        assertEquals(true, dispatched)
    }
}
