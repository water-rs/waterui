package dev.waterui.hydrolysis

import android.content.Context
import android.view.accessibility.AccessibilityNodeInfo
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.internal.DoNotInstrument

/**
 * The provider-local accessibility-focus contract: the focus ring is the
 * framework's drawable, painted by the view root's draw pass, so a focus
 * move or clear invalidates the host the way a real view does — the
 * session's GPU frame never paints it.
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    shadows = [ShadowNativeBridge::class],
    // The shadow can only replace `NativeBridge`'s natives on an
    // instrumented class.
    instrumentedPackages = ["dev.waterui.hydrolysis"],
)
@DoNotInstrument
class AccessibilityFocusInvalidationTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    @Before
    fun resetNative() {
        ShadowNativeBridge.reset()
        ShadowNativeBridge.treeJson = TREE_JSON
    }

    @Test
    fun anAccessibilityFocusMoveInvalidatesTheHost() {
        val host = HydrolysisHostView(context, HydrolysisSession(context, onCloseRequested = {}))
        val provider = host.accessibilityNodeProvider
        val shadowHost = shadowOf(host)

        shadowHost.clearWasInvalidated()
        assertTrue(
            "accessibility focus lands on the node",
            provider.performAction(BUTTON_ID, AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS, null),
        )
        assertTrue("the focus move must redraw the ring", shadowHost.wasInvalidated())

        shadowHost.clearWasInvalidated()
        provider.performAction(BUTTON_ID, AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS, null)
        assertFalse("a focus that does not move draws nothing new", shadowHost.wasInvalidated())

        provider.performAction(
            BUTTON_ID,
            AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS,
            null,
        )
        assertTrue("the focus clear must erase the ring", shadowHost.wasInvalidated())
    }

    private companion object {
        const val BUTTON_ID = 2

        /**
         * The minimal snapshot `ensureTree` parses: one window root holding
         * one focusable, clickable button. `actions` carries the accesskit
         * bit mask (Click | Focus = 3).
         */
        const val TREE_JSON = """
            {"update":{"nodes":[
                [0,{"role":"window","flags":0,"actions":0,
                    "properties":{"children":[2]}}],
                [2,{"role":"button","flags":0,"actions":3,
                    "properties":{"bounds":{"x0":0.0,"y0":0.0,"x1":48.0,"y1":48.0},"children":[]}}]
            ],"tree":{"root":0},"focus":0},"contentTypes":{}}
        """
    }
}
