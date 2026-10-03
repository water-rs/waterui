package dev.waterui.hydrolysis

import android.graphics.Rect
import android.os.Bundle
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import org.json.JSONArray
import org.json.JSONObject

/**
 * Serves `AccessibilityNodeInfo` for the self-drawn UI from the session's
 * serialized accesskit tree — there is no invisible shadow view tree.
 *
 * The native side publishes one merged `TreeUpdate` JSON per change and marks
 * it dirty through `onNativeAccessibilityTreeChanged`; this provider pulls it
 * lazily the next time a service asks for a node, so a frame costs a single
 * JNI read only when the tree actually changed.
 *
 * The JSON shape is accesskit's `TreeUpdate` serde: `nodes` is a list of
 * `[NodeId, Node]` pairs, `tree.root` names the root id and `focus` names the
 * focused node. Node fields used here are `role`, `name`/`value` (text) and
 * `children`/`filtered_child_ids` — anything absent reads as an empty list,
 * never as a fabricated node.
 */
internal class HydrolysisAccessibilityProvider(
    private val host: HydrolysisHostView,
    private val session: HydrolysisSession?,
) : AccessibilityNodeProvider() {

    private var dirty = true
    private var nodes = HashMap<Long, JSONObject>()
    private var childrenOf = HashMap<Long, List<Long>>()
    private var rootId = INVALID_ID
    private var focusedId = INVALID_ID

    fun notifyTreeChanged() {
        dirty = true
        host.sendAccessibilityEvent(
            android.view.accessibility.AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
        )
    }

    /** Pulls the session's snapshot once per publish. */
    private fun ensureTree() {
        if (!dirty) return
        dirty = false
        val sessionPtr = session?.nativePtr ?: return
        val json = NativeBridge.nativeAccessibilityTree(sessionPtr) ?: return
        val update = runCatching { JSONObject(json) }.getOrNull() ?: return
        val newNodes = HashMap<Long, JSONObject>()
        val newChildren = HashMap<Long, List<Long>>()
        val array = update.optJSONArray("nodes") ?: return
        for (i in 0 until array.length()) {
            val pair = array.optJSONArray(i) ?: continue
            val id = pair.optLong(0, INVALID_ID)
            val node = pair.optJSONObject(1) ?: continue
            newNodes[id] = node
            newChildren[id] = childIds(node)
        }
        nodes = newNodes
        childrenOf = newChildren
        rootId = update.optJSONObject("tree")?.optLong("root", INVALID_ID) ?: INVALID_ID
        focusedId = update.optLong("focus", INVALID_ID)
    }

    private fun childIds(node: JSONObject): List<Long> {
        val array =
            node.optJSONArray("children") ?: node.optJSONArray("filtered_child_ids")
            ?: return emptyList()
        return (0 until array.length()).map { array.optLong(it, INVALID_ID) }
    }

    override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
        ensureTree()
        val id = if (virtualViewId == HOST_ID) rootId else virtualViewId.toLong()
        val node = nodes[id] ?: return null
        val info = AccessibilityNodeInfo.obtain(host, virtualViewId)
        info.setSource(host, virtualViewId)
        info.isEnabled = true
        info.isVisibleToUser = true

        val role = node.optString("role")
        info.className = androidClassName(role)
        info.isClickable = role == "Button" || role == "Link" || role == "CheckBox" ||
            role == "ToggleButton" || role == "RadioButton" || role == "Tab"
        info.isCheckable = role == "CheckBox" || role == "ToggleButton" || role == "RadioButton"
        info.isEditable = role == "TextInput" || role == "TextField" || role == "MultilineTextInput"
        info.isScrollable = role == "ScrollView" || role == "List"
        info.isFocusable = role != "Label" && role != "StaticText" && role != "Image"
        info.isFocused = id == focusedId
        info.isAccessibilityFocused = id == focusedId

        val text = node.optString("value").ifEmpty { node.optString("name") }
        if (text.isNotEmpty()) info.text = text
        val name = node.optString("name")
        if (name.isNotEmpty()) info.contentDescription = name

        info.setBoundsInParent(nodeBounds(node))
        val location = IntArray(2)
        host.getLocationOnScreen(location)
        val bounds = nodeBounds(node)
        bounds.offset(location[0], location[1])
        info.setBoundsInScreen(bounds)

        if (info.isClickable) info.addAction(AccessibilityNodeInfo.ACTION_CLICK)
        if (info.isFocusable) {
            info.addAction(AccessibilityNodeInfo.ACTION_FOCUS)
            info.addAction(AccessibilityNodeInfo.ACTION_CLEAR_FOCUS)
        }
        if (info.isEditable) {
            info.addAction(AccessibilityNodeInfo.ACTION_SET_TEXT)
            info.addAction(AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY)
        }
        if (info.isScrollable) {
            info.addAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)
            info.addAction(AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD)
        }
        info.addAction(AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS)
        info.addAction(AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS)

        val children = childrenOf[id].orEmpty()
        for (childId in children) {
            if (childId != INVALID_ID) info.addChild(host, childId.toInt())
        }
        return info
    }

    override fun performAction(virtualViewId: Int, action: Int, arguments: Bundle?): Boolean {
        ensureTree()
        val id = if (virtualViewId == HOST_ID) rootId else virtualViewId.toLong()
        if (!nodes.containsKey(id)) return false
        val sessionPtr = session?.nativePtr ?: return false
        val value = arguments?.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE)
        val handled =
            NativeBridge.nativeAccessibilityAction(
                sessionPtr,
                id,
                action,
                value?.toString().orEmpty(),
            )
        if (handled) {
            host.sendAccessibilityEvent(
                android.view.accessibility.AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            )
        }
        return handled
    }

    override fun findFocus(focus: Int): AccessibilityNodeInfo? {
        ensureTree()
        if (focusedId == INVALID_ID) return null
        return createAccessibilityNodeInfo(focusedId.toInt())
    }

    private fun nodeBounds(node: JSONObject): Rect {
        val bounds = node.optJSONObject("bounds") ?: return Rect()
        return Rect(
            bounds.optDouble("x0", 0.0).toInt(),
            bounds.optDouble("y0", 0.0).toInt(),
            bounds.optDouble("x1", 0.0).toInt(),
            bounds.optDouble("y1", 0.0).toInt(),
        )
    }

    private fun androidClassName(role: String): String = when (role) {
        "Button" -> "android.widget.Button"
        "CheckBox" -> "android.widget.CheckBox"
        "RadioButton" -> "android.widget.RadioButton"
        "ToggleButton", "Switch" -> "android.widget.Switch"
        "TextInput", "TextField", "MultilineTextInput" -> "android.widget.EditText"
        "List" -> "android.widget.ListView"
        "ListItem" -> "android.widget.AdapterView"
        "ScrollView" -> "android.widget.ScrollView"
        "Image" -> "android.widget.ImageView"
        "ProgressIndicator" -> "android.widget.ProgressBar"
        "Slider" -> "android.widget.SeekBar"
        "Tab", "TabList" -> "android.widget.TabWidget"
        "Toolbar" -> "android.widget.Toolbar"
        "TitleBar" -> "android.app.ActionBar"
        else -> "android.view.View"
    }

    /** The editable roles the autofill adapter reports. */
    internal fun editableIds(): List<Long> {
        ensureTree()
        return nodes
            .filterValues { node ->
                node.optString("role") == "TextInput" || node.optString("role") == "TextField" ||
                    node.optString("role") == "MultilineTextInput"
            }
            .keys
            .toList()
    }

    internal fun nodeFor(id: Long): JSONObject? {
        ensureTree()
        return nodes[id]
    }

    private companion object {
        const val INVALID_ID = -1L
        const val HOST_ID = AccessibilityNodeProvider.HOST_VIEW_ID
    }
}
