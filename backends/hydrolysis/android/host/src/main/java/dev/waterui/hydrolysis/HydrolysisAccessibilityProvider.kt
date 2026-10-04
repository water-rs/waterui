package dev.waterui.hydrolysis

import android.graphics.Rect
import android.os.Bundle
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction
import android.view.accessibility.AccessibilityNodeInfo.CollectionInfo
import android.view.accessibility.AccessibilityNodeInfo.CollectionItemInfo
import android.view.accessibility.AccessibilityNodeInfo.RangeInfo
import android.view.accessibility.AccessibilityNodeProvider
import org.json.JSONObject

/**
 * Serves `AccessibilityNodeInfo` for the self-drawn UI from the session's
 * serialized accesskit tree — there is no invisible shadow view tree.
 *
 * The native side publishes one merged `TreeUpdate` JSON per semantic change
 * and marks it dirty through `onNativeAccessibilityTreeChanged`, whose
 * payload is the diffed event list this provider replays verbatim; the tree
 * itself is pulled lazily the next time a service asks for a node, so a
 * publish costs a JNI read only when a service is actually watching.
 *
 * The JSON shape is accesskit's serde: `nodes` is `[id, node]` pairs, `tree`
 * names the root id, `focus` the keyboard-focused node. Each node carries
 * `role` (camelCase), `actions`/`childActions`/`flags` (u32 bitmasks indexed
 * by accesskit enum ordinal) and `properties` (camelCase map: `children`,
 * `label`, `description`, `value`, `bounds` {x0,y0,x1,y1} logical units,
 * `numericValue`, `minNumericValue`, `maxNumericValue`, `numericValueStep`,
 * `toggled` "false"|"true"|"mixed", `expanded`, `selected`, `textSelection`,
 * `level`, `positionInSet`, `sizeOfSet`, `autocomplete`, `authorId`,
 * `language`, `url`, `hasPopup`, `live`, `customActions` [{id, description}]).
 *
 * Keyboard focus and accessibility focus stay distinct: `focus` reports the
 * former to `findFocus(FOCUS_INPUT)`; accessibility focus is tracked here —
 * `ACTION_ACCESSIBILITY_FOCUS` moves the local id and never reaches the
 * session, exactly as a focused decoration does on a real view.
 *
 * The transport back is the accesskit action *index*: the provider decodes a
 * node's `actions` bitmask, advertises the Android actions each bit implies,
 * and `performAction` echoes the index over JNI — no per-platform constant
 * table to keep in two languages.
 */
internal class HydrolysisAccessibilityProvider(
    private val host: HydrolysisHostView,
    private val session: HydrolysisSession?,
) : AccessibilityNodeProvider() {

    private var dirty = true
    private var nodes = HashMap<Long, JSONObject>()
    private var childrenOf = HashMap<Long, List<Long>>()
    private var parentOf = HashMap<Long, Long>()
    private var rootId = INVALID_ID
    /** Keyboard focus — the tree update's `focus` field. */
    private var keyboardFocusId = INVALID_ID
    /** Accessibility focus — owned by this provider, never sent to the session. */
    private var a11yFocusId = INVALID_ID
    /** The virtual node hover currently rests on, for explore-by-touch. */
    private var hoveredId = INVALID_ID

    /**
     * A publish landed: the native diff already decided which events the
     * change owes services — this replays them verbatim, so an unchanged
     * tree produces no event traffic at all (#246). `id` -1 addresses the
     * host view itself.
     */
    fun notifyTreeChanged(diffJson: String) {
        dirty = true
        // Building a virtual-source event queries this provider, which the
        // framework forbids while accessibility is off (uiautomator suppresses
        // services too) — replaying then would throw across the JNI boundary.
        if (!accessibilityEnabled()) return
        val events =
            runCatching { JSONObject(diffJson).optJSONArray("events") }.getOrNull() ?: return
        for (i in 0 until events.length()) {
            val entry = events.optJSONObject(i) ?: continue
            val type = entry.optInt("type")
            val mask = entry.optInt("mask")
            val id = entry.optLong("id", INVALID_ID)
            if (id == INVALID_ID || id > Int.MAX_VALUE) {
                val event = AccessibilityEvent.obtain(type)
                event.packageName = host.context.packageName
                event.contentChangeTypes = mask
                val parent = host.parent
                if (parent != null) {
                    parent.requestSendAccessibilityEvent(host, event)
                } else {
                    host.sendAccessibilityEventUnchecked(event)
                }
            } else {
                sendNodeEvent(id, type, mask)
            }
        }
    }

    private fun accessibilityEnabled(): Boolean {
        val manager = host.context.getSystemService(AccessibilityManager::class.java)
        return manager?.isEnabled == true
    }

    /** Pulls the session's snapshot once per publish. */
    private fun ensureTree() {
        if (!dirty) return
        dirty = false
        val sessionPtr = session?.nativePtr ?: return
        val json = NativeBridge.nativeAccessibilityTree(sessionPtr) ?: return
        val update = runCatching { JSONObject(json) }.getOrNull() ?: return
        val array = update.optJSONArray("nodes") ?: return
        val newNodes = HashMap<Long, JSONObject>()
        val newChildren = HashMap<Long, List<Long>>()
        for (i in 0 until array.length()) {
            val pair = array.optJSONArray(i) ?: continue
            val id = pair.optLong(0, INVALID_ID)
            val node = pair.optJSONObject(1) ?: continue
            // A Hidden node removes its whole subtree from traversal: the
            // hidden node never lands in `nodes`, so its children are never
            // reached from the root and `nodeInfo` skips missing children.
            if (hidden(node)) continue
            newNodes[id] = node
            newChildren[id] = childIds(node)
        }
        val newParents = HashMap<Long, Long>()
        for ((id, _) in newNodes) {
            for (childId in newChildren[id].orEmpty()) {
                if (newNodes.containsKey(childId)) newParents[childId] = id
            }
        }
        nodes = newNodes
        childrenOf = newChildren
        parentOf = newParents
        rootId = update.optJSONObject("tree")?.optLong("root", INVALID_ID) ?: INVALID_ID
        keyboardFocusId = update.optLong("focus", INVALID_ID)
        // A focus that pointed at a removed node drops cleanly.
        if (!nodes.containsKey(a11yFocusId)) a11yFocusId = INVALID_ID
        if (!nodes.containsKey(hoveredId)) hoveredId = INVALID_ID
        host.autofillSnapshotChanged()
    }

    private fun sendNodeEvent(nodeId: Long, type: Int, contentChangeTypes: Int = 0) {
        if (nodeId == INVALID_ID || nodeId > Int.MAX_VALUE) return
        if (!accessibilityEnabled()) return
        val event = AccessibilityEvent.obtain(type)
        event.setSource(host, nodeId.toInt())
        event.packageName = host.context.packageName
        event.contentChangeTypes = contentChangeTypes
        val parent = host.parent
        if (parent != null) {
            parent.requestSendAccessibilityEvent(host, event)
        } else {
            host.sendAccessibilityEventUnchecked(event)
        }
    }

    /**
     * Explore-by-touch dispatch from the host view. The native hit test
     * answers over the same tree this provider serves; the transitions it
     * reports become the HOVER_ENTER/EXIT pair TalkBack turns into
     * accessibility focus and an announcement.
     */
    internal fun dispatchHoverEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> {
                val id = hitTest(event.x, event.y)
                if (id != hoveredId) {
                    sendNodeEvent(hoveredId, AccessibilityEvent.TYPE_VIEW_HOVER_EXIT)
                    hoveredId = id
                    sendNodeEvent(hoveredId, AccessibilityEvent.TYPE_VIEW_HOVER_ENTER)
                }
            }
            MotionEvent.ACTION_HOVER_EXIT -> clearHovered()
        }
        return true
    }

    /** Pointer moved onto real content or left — drop the virtual hover. */
    internal fun clearHovered() {
        sendNodeEvent(hoveredId, AccessibilityEvent.TYPE_VIEW_HOVER_EXIT)
        hoveredId = INVALID_ID
    }

    /** The served node under the view-space point, resolved on the native side. */
    private fun hitTest(x: Float, y: Float): Long {
        val sessionPtr = session?.nativePtr ?: return INVALID_ID
        if (sessionPtr == 0L) return INVALID_ID
        val density = host.resources.displayMetrics.density
        return NativeBridge.nativeAccessibilityHitTest(sessionPtr, x / density, y / density)
    }

    private fun childIds(node: JSONObject): List<Long> {
        val array = props(node)?.optJSONArray("children") ?: return emptyList()
        val out = ArrayList<Long>(array.length())
        for (i in 0 until array.length()) {
            val id = array.optLong(i, INVALID_ID)
            if (id != INVALID_ID) out.add(id)
        }
        return out
    }

    private fun props(node: JSONObject): JSONObject? = node.optJSONObject("properties")

    private fun hidden(node: JSONObject): Boolean = flag(node, FLAG_HIDDEN)

    private fun flag(node: JSONObject, index: Int): Boolean =
        node.optLong("flags", 0L) and (1L shl index) != 0L

    private fun hasAction(node: JSONObject, index: Int): Boolean =
        node.optLong("actions", 0L) and (1L shl index) != 0L

    private fun isEditable(node: JSONObject): Boolean =
        when (node.optString("role")) {
            "textInput", "multilineTextInput", "passwordInput", "searchInput" -> true
            else -> false
        }

    private fun isRange(node: JSONObject): Boolean =
        when (node.optString("role")) {
            "slider", "spinButton", "progressIndicator", "progressBar",
            "meter", "levelIndicator", "stepper" -> true
            else -> false
        }

    override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
        ensureTree()
        if (virtualViewId == HOST_ID) return hostNodeInfo()
        val id = virtualViewId.toLong()
        val node = nodes[id]
        if (node == null) return null
        return nodeInfo(id, node)
    }

    /**
     * The host node: it owns the semantic root as a virtual child plus every
     * mounted platform view as a *real* child — the mounted view's own
     * accessibility tree is reached by traversal without a duplicate node.
     */
    private fun hostNodeInfo(): AccessibilityNodeInfo {
        val info = AccessibilityNodeInfo.obtain()
        // The framework uses this node as the host's own node, so it has to
        // carry the real view's bounds/flags — an empty bounds rect marks it
        // invisible and prunes the whole virtual subtree.
        host.onInitializeAccessibilityNodeInfo(info)
        // onInitialize fills bounds/flags but does NOT set the source node
        // id; a client-side getChild() refuses to query children of a node
        // whose source is UNDEFINED, so every virtual child resolved to null.
        info.setSource(host, AccessibilityNodeProvider.HOST_VIEW_ID)
        if (nodes.containsKey(rootId)) {
            info.addChild(host, rootId.toInt())
        }
        for (child in host.platformViewRegistry.accessibilityChildren()) {
            info.addChild(child)
        }
        return info
    }

    private fun nodeInfo(id: Long, node: JSONObject): AccessibilityNodeInfo {
        val info = AccessibilityNodeInfo.obtain(host, id.toInt())
        info.packageName = host.context.packageName

        val properties = props(node)
        val role = node.optString("role")
        // The parent link is what lets a service walk up from a virtual node
        // — linear traversal and ancestor queries need it, and without it
        // TalkBack cannot rank the node against its siblings.
        val parent = parentOf[id]
        if (parent != null && parent <= Int.MAX_VALUE) {
            info.setParent(host, parent.toInt())
        } else {
            info.setParent(host)
        }
        info.className = androidClassName(role)

        val label = properties?.optString("label").orEmpty()
        val value = properties?.optString("value").orEmpty()
        val description = properties?.optString("description").orEmpty()
        // `value` is what the node contains; `label` names it for traversal.
        if (value.isNotEmpty()) info.text = value
        if (label.isNotEmpty()) info.contentDescription = label
        if (info.text.isNullOrEmpty() && label.isNotEmpty()) info.text = label
        if (description.isNotEmpty()) {
            info.contentDescription =
                listOfNotNull(info.contentDescription, description).joinToString("\n")
        }
        if (isEditable(node)) {
            info.isEditable = true
            if (role == "passwordInput") info.isPassword = true
        }
        properties?.optString("placeholder")?.takeIf { it.isNotEmpty() }?.let {
            info.hintText = it
        }

        info.isEnabled = !flag(node, FLAG_DISABLED)
        info.isVisibleToUser = true
        info.isClickable = hasAction(node, AK_CLICK) || role in CLICKABLE_ROLES
        info.isFocusable =
            hasAction(node, AK_FOCUS) || info.isEditable || isRange(node)
        info.isFocused = id == keyboardFocusId
        info.isAccessibilityFocused = id == a11yFocusId
        info.isScreenReaderFocusable =
            info.isFocusable || info.isClickable || !info.contentDescription.isNullOrEmpty()
        info.isContextClickable = hasAction(node, AK_SHOW_CONTEXT_MENU)
        if (properties != null && properties.optBoolean("selected", false)) {
            info.isSelected = true
        }
        when (properties?.optString("toggled")) {
            "true", "false" -> {
                info.isCheckable = true
                info.isChecked = properties.optString("toggled") == "true"
            }
            "mixed" -> info.isCheckable = true
        }
        if (properties != null && properties.has("expanded")) {
            info.stateDescription =
                if (properties.optBoolean("expanded")) "Expanded" else "Collapsed"
        }
        if (properties != null && properties.has("level")) info.isHeading = true

        val bounds = boundsRect(properties)
        info.setBoundsInParent(bounds)
        val screenBounds = Rect(bounds)
        val location = IntArray(2)
        host.getLocationOnScreen(location)
        screenBounds.offset(location[0], location[1])
        info.setBoundsInScreen(screenBounds)

        if (isRange(node)) {
            val min = properties?.optDouble("minNumericValue", 0.0) ?: 0.0
            val max = properties?.optDouble("maxNumericValue", 1.0) ?: 1.0
            val current = properties?.optDouble("numericValue", 0.0) ?: 0.0
            info.rangeInfo =
                RangeInfo.obtain(
                    RangeInfo.RANGE_TYPE_FLOAT,
                    min.toFloat(),
                    max.toFloat(),
                    current.toFloat(),
                )
        }
        val position = properties?.optInt("positionInSet", -1) ?: -1
        val size = properties?.optInt("sizeOfSet", -1) ?: -1
        if (role == "list" || role == "grid" || role == "table" || role == "listBox") {
            info.setCollectionInfo(
                CollectionInfo.obtain(if (size > 0) size else -1, -1, false)
            )
        }
        if (role == "listItem" || role == "listBoxOption" || role == "option" ||
            role == "cell" || role == "row"
        ) {
            info.setCollectionItemInfo(
                CollectionItemInfo.obtain(
                    -1,
                    0,
                    if (position >= 0) position else -1,
                    0,
                    false,
                )
            )
        }
        if (properties != null && properties.has("textSelection")) {
            val selection = properties.optJSONObject("textSelection")
            val anchor = selection?.optJSONObject("anchor")?.optInt("characterIndex")
            val focus = selection?.optJSONObject("focus")?.optInt("characterIndex")
            if (anchor != null && focus != null) info.setTextSelection(anchor, focus)
        }

        advertiseActions(info, node)

        for (childId in childrenOf[id].orEmpty()) {
            if (nodes.containsKey(childId)) info.addChild(host, childId.toInt())
        }
        return info
    }

    /** Maps the accesskit `actions` bitmask onto advertised Android actions. */
    private fun advertiseActions(info: AccessibilityNodeInfo, node: JSONObject) {
        if (hasAction(node, AK_CLICK)) info.addAction(ACTION_CLICK)
        if (hasAction(node, AK_FOCUS)) info.addAction(ACTION_FOCUS)
        if (hasAction(node, AK_BLUR)) info.addAction(ACTION_CLEAR_FOCUS)
        if (hasAction(node, AK_EXPAND)) info.addAction(AccessibilityAction.ACTION_EXPAND)
        if (hasAction(node, AK_COLLAPSE)) info.addAction(AccessibilityAction.ACTION_COLLAPSE)
        if (hasAction(node, AK_INCREMENT) || hasAction(node, AK_DECREMENT)) {
            info.addAction(ACTION_SCROLL_FORWARD)
            info.addAction(ACTION_SCROLL_BACKWARD)
            if (hasAction(node, AK_SET_VALUE)) {
                info.addAction(AccessibilityAction.ACTION_SET_PROGRESS)
            }
        }
        if (hasAction(node, AK_SCROLL_DOWN) || hasAction(node, AK_SCROLL_RIGHT)) {
            info.addAction(ACTION_SCROLL_FORWARD)
            info.isScrollable = true
        }
        if (hasAction(node, AK_SCROLL_UP) || hasAction(node, AK_SCROLL_LEFT)) {
            info.addAction(ACTION_SCROLL_BACKWARD)
            info.isScrollable = true
        }
        if (hasAction(node, AK_SCROLL_UP)) {
            info.addAction(AccessibilityAction.ACTION_SCROLL_UP)
        }
        if (hasAction(node, AK_SCROLL_DOWN)) {
            info.addAction(AccessibilityAction.ACTION_SCROLL_DOWN)
        }
        if (hasAction(node, AK_SCROLL_LEFT)) {
            info.addAction(AccessibilityAction.ACTION_SCROLL_LEFT)
        }
        if (hasAction(node, AK_SCROLL_RIGHT)) {
            info.addAction(AccessibilityAction.ACTION_SCROLL_RIGHT)
        }
        if (hasAction(node, AK_SCROLL_INTO_VIEW)) {
            info.addAction(AccessibilityAction.ACTION_SHOW_ON_SCREEN)
        }
        if (hasAction(node, AK_SHOW_TOOLTIP)) {
            info.addAction(AccessibilityAction.ACTION_SHOW_TOOLTIP)
        }
        if (hasAction(node, AK_HIDE_TOOLTIP)) {
            info.addAction(AccessibilityAction.ACTION_HIDE_TOOLTIP)
        }
        if (hasAction(node, AK_SHOW_CONTEXT_MENU)) {
            info.addAction(AccessibilityAction.ACTION_CONTEXT_CLICK)
        }
        if (isEditable(node) && hasAction(node, AK_SET_VALUE)) {
            info.addAction(ACTION_SET_TEXT)
            info.addAction(ACTION_NEXT_AT_MOVEMENT_GRANULARITY)
            info.addAction(ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY)
            // Selection is intrinsic to the editing session the text actions
            // run over — it needs no separate accesskit bit.
            info.addAction(ACTION_SET_SELECTION)
            info.addAction(AccessibilityAction.ACTION_IME_ENTER)
        }
        // Named custom actions ride in as `customActions` entries; the id
        // they advertise encodes their position so `performAction` can echo
        // the accesskit index back.
        props(node)?.optJSONArray("customActions")?.let { custom ->
            for (i in 0 until custom.length()) {
                val entry = custom.optJSONObject(i) ?: continue
                val label = entry.optString("description")
                if (label.isEmpty()) continue
                info.addAction(AccessibilityAction(CUSTOM_ACTION_BASE + i, label))
            }
        }
        if (info.isFocusable || info.isClickable) {
            info.addAction(ACTION_ACCESSIBILITY_FOCUS)
            info.addAction(ACTION_CLEAR_ACCESSIBILITY_FOCUS)
        }
    }

    /**
     * `bounds` arrives in window logical units — the same space input events
     * leave through after `density` division. Services want physical px.
     */
    private fun boundsRect(properties: JSONObject?): Rect {
        val bounds = properties?.optJSONObject("bounds") ?: return Rect()
        val density = host.resources.displayMetrics.density
        fun px(value: Double) = (value * density).toInt()
        return Rect(
            px(bounds.optDouble("x0", 0.0)),
            px(bounds.optDouble("y0", 0.0)),
            px(bounds.optDouble("x1", 0.0)),
            px(bounds.optDouble("y1", 0.0)),
        )
    }

    override fun performAction(virtualViewId: Int, action: Int, arguments: Bundle?): Boolean {
        ensureTree()
        if (virtualViewId == HOST_ID) return false
        val id = virtualViewId.toLong()
        val node = nodes[id] ?: return false

        // Accessibility focus is provider-local state — keyboard focus is the
        // session's, per the plan's distinction.
        when (action) {
            AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS -> {
                if (a11yFocusId == id) return true
                a11yFocusId = id
                sendNodeEvent(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED)
                sendNodeEvent(
                    id,
                    AccessibilityEvent.TYPE_VIEW_TEXT_TRAVERSED_AT_MOVEMENT_GRANULARITY,
                )
                return true
            }
            AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> {
                if (a11yFocusId == id) {
                    a11yFocusId = INVALID_ID
                    sendNodeEvent(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED)
                }
                return true
            }
            AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY,
            AccessibilityNodeInfo.ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY,
            -> {
                // Text-cursor traversal needs text-selection control the
                // renderer does not expose; the node's text is on the info.
                return false
            }
        }

        val mapped = mapAction(node, action, arguments) ?: return false
        val sessionPtr = session?.nativePtr ?: return false
        val handled =
            NativeBridge.nativeAccessibilityAction(
                sessionPtr,
                id,
                mapped.index,
                mapped.arg1,
                mapped.arg2,
                mapped.text.orEmpty(),
                mapped.numeric ?: Double.NaN,
            )
        if (handled) {
            host.sendAccessibilityEvent(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED)
        }
        return handled
    }

    /**
     * (accesskit action index, text payload, numeric payload, selection
     * bounds as UTF-16 units — -1 when the action carries none).
     */
    private data class MappedAction(
        val index: Int,
        val text: String?,
        val numeric: Double?,
        val arg1: Int = -1,
        val arg2: Int = -1,
    )

    private fun mapAction(
        node: JSONObject,
        action: Int,
        arguments: Bundle?,
    ): MappedAction? {
        when (action) {
            AccessibilityNodeInfo.ACTION_CLICK ->
                if (hasAction(node, AK_CLICK)) return MappedAction(AK_CLICK, null, null)
            AccessibilityNodeInfo.ACTION_LONG_CLICK ->
                if (hasAction(node, AK_SHOW_CONTEXT_MENU)) {
                    return MappedAction(AK_SHOW_CONTEXT_MENU, null, null)
                }
            AccessibilityNodeInfo.ACTION_FOCUS ->
                if (hasAction(node, AK_FOCUS)) return MappedAction(AK_FOCUS, null, null)
            AccessibilityNodeInfo.ACTION_CLEAR_FOCUS ->
                if (hasAction(node, AK_BLUR)) return MappedAction(AK_BLUR, null, null)
            AccessibilityAction.ACTION_EXPAND.id ->
                if (hasAction(node, AK_EXPAND)) return MappedAction(AK_EXPAND, null, null)
            AccessibilityAction.ACTION_COLLAPSE.id ->
                if (hasAction(node, AK_COLLAPSE)) return MappedAction(AK_COLLAPSE, null, null)
            AccessibilityNodeInfo.ACTION_SCROLL_FORWARD -> {
                if (hasAction(node, AK_INCREMENT)) return MappedAction(AK_INCREMENT, null, null)
                if (hasAction(node, AK_SCROLL_DOWN)) {
                    return MappedAction(AK_SCROLL_DOWN, null, null)
                }
                if (hasAction(node, AK_SCROLL_RIGHT)) {
                    return MappedAction(AK_SCROLL_RIGHT, null, null)
                }
            }
            AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD -> {
                if (hasAction(node, AK_DECREMENT)) return MappedAction(AK_DECREMENT, null, null)
                if (hasAction(node, AK_SCROLL_UP)) return MappedAction(AK_SCROLL_UP, null, null)
                if (hasAction(node, AK_SCROLL_LEFT)) {
                    return MappedAction(AK_SCROLL_LEFT, null, null)
                }
            }
            AccessibilityAction.ACTION_SCROLL_UP.id ->
                if (hasAction(node, AK_SCROLL_UP)) return MappedAction(AK_SCROLL_UP, null, null)
            AccessibilityAction.ACTION_SCROLL_DOWN.id ->
                if (hasAction(node, AK_SCROLL_DOWN)) {
                    return MappedAction(AK_SCROLL_DOWN, null, null)
                }
            AccessibilityAction.ACTION_SCROLL_LEFT.id ->
                if (hasAction(node, AK_SCROLL_LEFT)) {
                    return MappedAction(AK_SCROLL_LEFT, null, null)
                }
            AccessibilityAction.ACTION_SCROLL_RIGHT.id ->
                if (hasAction(node, AK_SCROLL_RIGHT)) {
                    return MappedAction(AK_SCROLL_RIGHT, null, null)
                }
            AccessibilityAction.ACTION_SHOW_ON_SCREEN.id ->
                if (hasAction(node, AK_SCROLL_INTO_VIEW)) {
                    return MappedAction(AK_SCROLL_INTO_VIEW, null, null)
                }
            AccessibilityAction.ACTION_SHOW_TOOLTIP.id ->
                if (hasAction(node, AK_SHOW_TOOLTIP)) {
                    return MappedAction(AK_SHOW_TOOLTIP, null, null)
                }
            AccessibilityAction.ACTION_HIDE_TOOLTIP.id ->
                if (hasAction(node, AK_HIDE_TOOLTIP)) {
                    return MappedAction(AK_HIDE_TOOLTIP, null, null)
                }
            AccessibilityAction.ACTION_CONTEXT_CLICK.id ->
                if (hasAction(node, AK_SHOW_CONTEXT_MENU)) {
                    return MappedAction(AK_SHOW_CONTEXT_MENU, null, null)
                }
            AccessibilityAction.ACTION_SET_PROGRESS.id -> {
                if (hasAction(node, AK_SET_VALUE) && isRange(node)) {
                    val value =
                        arguments?.getFloat(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_PROGRESS_VALUE,
                            Float.NaN,
                        )
                    if (value != null && !value.isNaN()) {
                        return MappedAction(AK_SET_VALUE, null, value.toDouble())
                    }
                }
            }
            AccessibilityNodeInfo.ACTION_SET_TEXT -> {
                if (hasAction(node, AK_SET_VALUE) && isEditable(node)) {
                    val value =
                        arguments?.getCharSequence(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                        )
                    if (value != null) {
                        return MappedAction(AK_SET_VALUE, value.toString(), null)
                    }
                }
            }
            AccessibilityNodeInfo.ACTION_SET_SELECTION -> {
                if (isEditable(node)) {
                    val start =
                        arguments?.getInt(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT,
                            -1,
                        ) ?: -1
                    val end =
                        arguments?.getInt(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT,
                            -1,
                        ) ?: -1
                    if (start >= 0 && end >= 0) {
                        return MappedAction(
                            AK_SET_TEXT_SELECTION,
                            null,
                            null,
                            arg1 = start,
                            arg2 = end,
                        )
                    }
                }
            }
            AccessibilityAction.ACTION_IME_ENTER.id ->
                if (isEditable(node) && hasAction(node, AK_FOCUS)) {
                    return MappedAction(AK_FOCUS, null, null)
                }
            else -> {
                if (action >= CUSTOM_ACTION_BASE && hasAction(node, AK_CUSTOM_ACTION)) {
                    val index = action - CUSTOM_ACTION_BASE
                    val custom = props(node)?.optJSONArray("customActions")
                    if (custom != null && index < custom.length()) {
                        return MappedAction(AK_CUSTOM_ACTION, null, index.toDouble())
                    }
                }
            }
        }
        return null
    }

    override fun findFocus(focus: Int): AccessibilityNodeInfo? {
        ensureTree()
        val id =
            when (focus) {
                AccessibilityNodeInfo.FOCUS_INPUT -> keyboardFocusId
                AccessibilityNodeInfo.FOCUS_ACCESSIBILITY -> a11yFocusId
                else -> return null
            }
        if (id == INVALID_ID || !nodes.containsKey(id)) return null
        return createAccessibilityNodeInfo(id.toInt())
    }

    // ------------------------------------------------------------------
    // Autofill reads the same tree — editable nodes and their geometry in
    // host-view physical px.

    internal data class EditableNode(
        val id: Long,
        val label: String,
        val value: String,
        val sensitive: Boolean,
        val bounds: Rect,
        val autofillHints: Array<String>,
    )

    /** The editable nodes, in tree order, for the autofill structure. */
    internal fun editableNodes(): List<EditableNode> {
        ensureTree()
        val out = ArrayList<EditableNode>()
        val stack = ArrayDeque<Long>()
        if (nodes.containsKey(rootId)) stack.add(rootId)
        while (stack.isNotEmpty()) {
            val id = stack.removeLast()
            val node = nodes[id] ?: continue
            if (isEditable(node)) {
                val properties = props(node)
                out.add(
                    EditableNode(
                        id = id,
                        label = properties?.optString("label").orEmpty(),
                        value = properties?.optString("value").orEmpty(),
                        sensitive = node.optString("role") == "passwordInput",
                        bounds = boundsRect(properties),
                        autofillHints = autofillHints(node),
                    )
                )
            }
            val children = childrenOf[id].orEmpty()
            for (i in children.indices.reversed()) stack.add(children[i])
        }
        return out
    }

    internal fun focusedEditableId(): Long {
        ensureTree()
        val node = nodes[keyboardFocusId] ?: return INVALID_ID
        return if (isEditable(node)) keyboardFocusId else INVALID_ID
    }

    internal fun textOf(id: Long): String? {
        ensureTree()
        val node = nodes[id] ?: return null
        if (!isEditable(node)) return null
        return props(node)?.optString("value").orEmpty()
    }

    /** The node's bounds in screen px, matching `setBoundsInScreen`. */
    internal fun editableScreenBounds(id: Long): Rect? {
        ensureTree()
        val node = nodes[id] ?: return null
        if (!isEditable(node)) return null
        val bounds = boundsRect(props(node))
        val location = IntArray(2)
        host.getLocationOnScreen(location)
        bounds.offset(location[0], location[1])
        return bounds
    }

    internal fun applyAutofillValue(id: Long, text: String): Boolean {
        ensureTree()
        val node = nodes[id] ?: return false
        if (!isEditable(node) || !hasAction(node, AK_SET_VALUE)) return false
        val sessionPtr = session?.nativePtr ?: return false
        return NativeBridge.nativeAccessibilityAction(
            sessionPtr,
            id,
            AK_SET_VALUE,
            -1,
            -1,
            text,
            Double.NaN,
        )
    }

    /** Autofill has no hints of its own here — infer from role and label. */
    private fun autofillHints(node: JSONObject): Array<String> {
        val label = props(node)?.optString("label").orEmpty().lowercase()
        val role = node.optString("role")
        val hint =
            when {
                role == "passwordInput" -> View.AUTOFILL_HINT_PASSWORD
                "email" in label -> View.AUTOFILL_HINT_EMAIL_ADDRESS
                "phone" in label -> View.AUTOFILL_HINT_PHONE
                "username" in label || "user name" in label ->
                    View.AUTOFILL_HINT_USERNAME
                "name" in label -> View.AUTOFILL_HINT_NAME
                "postal" in label || "zip" in label -> View.AUTOFILL_HINT_POSTAL_CODE
                "card" in label -> View.AUTOFILL_HINT_CREDIT_CARD_NUMBER
                else -> null
            }
        return hint?.let { arrayOf(it) } ?: emptyArray()
    }

    private fun androidClassName(role: String): String =
        when (role) {
            "button" -> "android.widget.Button"
            "checkBox", "checkbox", "menuItemCheckBox", "menuItemCheckbox" ->
                "android.widget.CheckBox"
            "radioButton", "menuItemRadio" -> "android.widget.RadioButton"
            "switch", "toggleButton" -> "android.widget.Switch"
            "textInput", "multilineTextInput", "passwordInput", "searchInput" ->
                "android.widget.EditText"
            "list", "grid", "table", "listBox" -> "android.widget.AbsListView"
            "listItem", "listBoxOption", "option", "cell" ->
                "android.widget.AdapterView"
            "scrollView" -> "android.widget.ScrollView"
            "image" -> "android.widget.ImageView"
            "progressIndicator", "progressBar" -> "android.widget.ProgressBar"
            "slider", "spinButton" -> "android.widget.SeekBar"
            "stepper" -> "android.widget.NumberPicker"
            "tab", "tabList" -> "android.widget.TabWidget"
            "toolbar" -> "android.widget.Toolbar"
            "titleBar" -> "android.app.ActionBar"
            "link" -> "android.widget.TextView"
            "dialog" -> "android.app.Dialog"
            "menu", "menuBar" -> "android.widget.Menu"
            "menuItem" -> "android.view.MenuItem"
            else -> "android.view.View"
        }

    private companion object {
        const val INVALID_ID = -1L
        const val HOST_ID = AccessibilityNodeProvider.HOST_VIEW_ID

        /** accesskit Action ordinals — the bitmask indices on the wire. */
        const val AK_CLICK = 0
        const val AK_FOCUS = 1
        const val AK_BLUR = 2
        const val AK_COLLAPSE = 3
        const val AK_EXPAND = 4
        const val AK_CUSTOM_ACTION = 5
        const val AK_DECREMENT = 6
        const val AK_INCREMENT = 7
        const val AK_HIDE_TOOLTIP = 8
        const val AK_SHOW_TOOLTIP = 9
        const val AK_REPLACE_SELECTED_TEXT = 10
        const val AK_SCROLL_DOWN = 11
        const val AK_SCROLL_LEFT = 12
        const val AK_SCROLL_RIGHT = 13
        const val AK_SCROLL_UP = 14
        const val AK_SCROLL_INTO_VIEW = 15
        const val AK_SET_TEXT_SELECTION = 18
        const val AK_SET_VALUE = 20
        const val AK_SHOW_CONTEXT_MENU = 21

        /** accesskit Flag ordinals. */
        const val FLAG_HIDDEN = 0
        const val FLAG_DISABLED = 9

        /** Advertised ids for accesskit custom actions start here. */
        const val CUSTOM_ACTION_BASE = 0x01000000

        const val ACTION_CLICK = AccessibilityNodeInfo.ACTION_CLICK
        const val ACTION_FOCUS = AccessibilityNodeInfo.ACTION_FOCUS
        const val ACTION_CLEAR_FOCUS = AccessibilityNodeInfo.ACTION_CLEAR_FOCUS
        const val ACTION_SCROLL_FORWARD = AccessibilityNodeInfo.ACTION_SCROLL_FORWARD
        const val ACTION_SCROLL_BACKWARD = AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD
        const val ACTION_SET_TEXT = AccessibilityNodeInfo.ACTION_SET_TEXT
        const val ACTION_SET_SELECTION = AccessibilityNodeInfo.ACTION_SET_SELECTION

        const val ACTION_NEXT_AT_MOVEMENT_GRANULARITY =
            AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY
        const val ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY =
            AccessibilityNodeInfo.ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY
        const val ACTION_ACCESSIBILITY_FOCUS =
            AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS
        const val ACTION_CLEAR_ACCESSIBILITY_FOCUS =
            AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS

        val CLICKABLE_ROLES =
            setOf(
                "button", "link", "checkBox", "checkbox", "radioButton", "switch",
                "toggleButton", "tab", "menuItem", "menuItemCheckBox",
                "menuItemCheckbox", "menuItemRadio", "listBoxOption", "option",
            )
    }
}
