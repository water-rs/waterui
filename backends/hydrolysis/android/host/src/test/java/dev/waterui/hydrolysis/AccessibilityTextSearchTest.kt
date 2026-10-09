package dev.waterui.hydrolysis

import android.content.Context
import android.view.accessibility.AccessibilityNodeProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.internal.DoNotInstrument

/**
 * `findAccessibilityNodeInfosByText` against a shadowed publish: a query
 * against a node's `text` and one against its `contentDescription` each
 * find the virtual node serving them — case-insensitively, as
 * `View.findViewsWithText` defines it — and a non-matching query finds
 * nothing.
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    shadows = [ShadowNativeBridge::class],
    instrumentedPackages = ["dev.waterui.hydrolysis"],
)
@DoNotInstrument
class AccessibilityTextSearchTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    @Before
    fun resetNative() {
        ShadowNativeBridge.reset()
        ShadowNativeBridge.accessibilityTree = TREE_JSON
    }

    /** A provider mounted over [TREE_JSON], reached the way the platform does. */
    private fun provider(): HydrolysisAccessibilityProvider {
        val session = HydrolysisSession(context, onCloseRequested = {})
        return HydrolysisHostView(context, session).accessibilityNodeProvider
            as HydrolysisAccessibilityProvider
    }

    @Test
    fun aTextQueryFindsTheNodeServingThatText() {
        val found =
            provider().findAccessibilityNodeInfosByText(
                "newsletter",
                AccessibilityNodeProvider.HOST_VIEW_ID,
            )
        assertEquals(1, found.size)
        assertEquals("Newsletter: false", found[0].text.toString())
    }

    @Test
    fun aContentDescriptionQueryFindsTheNodeServingIt() {
        val found =
            provider().findAccessibilityNodeInfosByText(
                "LOGO",
                AccessibilityNodeProvider.HOST_VIEW_ID,
            )
        assertEquals(1, found.size)
        assertEquals("Company logo", found[0].contentDescription.toString())
    }

    @Test
    fun aNonMatchingQueryFindsNothing() {
        assertTrue(
            provider()
                .findAccessibilityNodeInfosByText(
                    "absent",
                    AccessibilityNodeProvider.HOST_VIEW_ID,
                )
                .isEmpty(),
        )
    }

    private companion object {

        /** The serde envelope `nativeAccessibilityTree` hands the provider. */
        const val TREE_JSON = """
{
  "update": {
    "nodes": [
      [
        7,
        {
          "role": "genericContainer",
          "actions": 0,
          "childActions": 0,
          "flags": 0,
          "properties": {
            "children": [8, 9],
            "bounds": {"x0": 0, "y0": 0, "x1": 400, "y1": 200}
          }
        }
      ],
      [
        8,
        {
          "role": "label",
          "actions": 0,
          "childActions": 0,
          "flags": 0,
          "properties": {
            "value": "Newsletter: false",
            "bounds": {"x0": 0, "y0": 0, "x1": 200, "y1": 24}
          }
        }
      ],
      [
        9,
        {
          "role": "image",
          "actions": 0,
          "childActions": 0,
          "flags": 0,
          "properties": {
            "description": "Company logo",
            "bounds": {"x0": 0, "y0": 40, "x1": 64, "y1": 104}
          }
        }
      ]
    ],
    "tree": {"root": 7},
    "focus": 7
  },
  "contentTypes": {}
}
"""
    }
}