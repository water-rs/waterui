package dev.waterui.hydrolysis

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.Rect
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.core.view.isVisible
import org.json.JSONArray

/**
 * The embedded-native-view registry: a transparent [FrameLayout] laid out on
 * top of the GPU band inside the host view. The session publishes the whole
 * placement set as one JSON frame —
 * `[{id,kind,x,y,width,height,clip,order,visible}]` in *window logical units*,
 * the same space the accessibility bounds and input events use; this registry
 * scales by the display density.
 *
 * A placement's slot is a `FrameLayout` that owns the mounted view's event
 * and clip ownership: the slot fills the frame rect, `clipBounds` applies the
 * clip the renderer computed, and the slot is clickable so a touch the inner
 * view declines stays on the platform view's footprint instead of falling
 * through to the WaterUI scene.
 *
 * Z-order follows the renderer's `order`: the slot list is re-sorted to match
 * on every publish. This is a *single* GPU band — platform views always stack
 * above GPU-drawn content; splitting GPU content around a native child is the
 * Cherenkov interleaved-bands work (hydrolysis#205), not this step.
 *
 * A placement with no registered factory mounts nothing and keeps tracking —
 * the frame applies the moment `registerFactory` lands. A placement whose
 * *kind* has no factory is a programmer error: the app declared a native view
 * it cannot build, so [Slot] creation throws naming the kind.
 */
class PlatformViewRegistry internal constructor(
    private val context: Context,
    private val session: HydrolysisSession?,
    private val hostView: HydrolysisHostView?,
) {
    /**
     * The overlay container the host view adds as its topmost child. A
     * child focus gain or loss inside it requests the frame that pulls
     * [focusInside]: `requestFrame` only posts a Choreographer callback,
     * so the hook can never re-enter the session the way a pushed native
     * report did.
     *
     * The registry keeps its host view past the session's unbind, and the
     * unbind itself requests a frame here: freeing a focused instance from
     * its slot clears child focus. Those requests go to the session's
     * scheduler, which the teardown stops, so a frame posted on the live
     * session is removed with it and a later one posts nothing. That stop
     * is what makes this reference safe; nothing here checks the session.
     */
    internal val container: FrameLayout =
        object : FrameLayout(context) {
            init {
                // Ordered dispatch visits the gesture's chosen child first
                // (last in forward drawing order).
                isChildrenDrawingOrderEnabled = true
            }

            /**
             * Whether the gesture in progress belongs to a platform view.
             * Latched at ACTION_DOWN: the topmost ordered entry under the
             * pointer keeps the whole gesture even when the order changes
             * underneath it, matching ViewGroup's single-target dispatch.
             */
            private var nativeGesture = true

            override fun requestChildFocus(child: View?, focused: View?) {
                super.requestChildFocus(child, focused)
                hostView?.requestFrame()
            }

            override fun clearChildFocus(child: View?) {
                super.clearChildFocus(child)
                hostView?.requestFrame()
            }

            override fun dispatchDraw(canvas: Canvas) {
                val content = orderedContent ?: return super.dispatchDraw(canvas)
                // The painter re-records every bound view's portal before the
                // root draws — the views themselves draw through their
                // owning RenderNode's place in the retained tree.
                content.preparePlatformViews(platformViewRecorder)
                content.drawBackground(canvas)
                for (index in 0 until content.entryCount) {
                    if (content.isPlatformView(index)) {
                        val id = content.platformViewId(index)
                        val view =
                            slots[id]?.view
                                ?: error(
                                    "hydrolysis: ordered content names unmounted " +
                                        "platform view $id",
                                )
                        if (view.isVisible) drawChild(canvas, view, drawingTime)
                    } else {
                        content.drawEntry(canvas, index)
                    }
                }
            }

            /** The platform view the last DOWN latched — gesture ownership
             *  stays with it through MOVE/UP even when the order changes. */
            private var nativeTarget: View? = null

            override fun getChildDrawingOrder(childCount: Int, drawingPosition: Int): Int {
                val target = nativeTarget
                val chosen = target?.let { indexOfChild(it) } ?: -1
                if (chosen < 0) return drawingPosition
                // The chosen target visits last in forward order, so it is
                // the first child Android's reversed touch dispatch asks.
                if (drawingPosition == childCount - 1) return chosen
                return if (drawingPosition >= chosen) drawingPosition + 1 else drawingPosition
            }

            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                try {
                    return super.dispatchTouchEvent(event)
                } finally {
                    if (event.actionMasked == MotionEvent.ACTION_UP ||
                        event.actionMasked == MotionEvent.ACTION_CANCEL
                    ) {
                        nativeTarget = null
                    }
                }
            }

            override fun onInterceptTouchEvent(event: MotionEvent): Boolean {
                val content = orderedContent ?: return super.onInterceptTouchEvent(event)
                if (event.actionMasked == MotionEvent.ACTION_DOWN) {
                    // The retained tree answers who owns the point — alpha,
                    // transforms, clips and scroll already composed there;
                    // the slot's own matrix/clip is the visible-geometry check.
                    val id = content.platformViewAt(event.x, event.y)
                    nativeGesture = id != null && slotAt(id, event.x, event.y) != null
                    nativeTarget = if (nativeGesture) slots[id]?.view else null
                }
                return !nativeGesture || super.onInterceptTouchEvent(event)
            }

            @SuppressLint("ClickableViewAccessibility")
            override fun onTouchEvent(event: MotionEvent): Boolean {
                if (orderedContent != null && !nativeGesture) {
                    return hostView?.onTouchEvent(event) ?: false
                }
                return super.onTouchEvent(event)
            }
        }.apply {
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            clipChildren = true
        }

    private val factories = HashMap<String, (Context) -> View>()
    private val slots = LinkedHashMap<Long, Slot>()

    /**
     * The [OrderedContent] recorder: records the mounted view's own display
     * list into the painter's portal canvas — the real child's `onDraw`,
     * no bitmap or readback.
     */
    private val platformViewRecorder =
        PlatformViewRecorder { id, canvas, width, height, transform ->
            val slot =
                slots[id]
                    ?: error(
                        "hydrolysis: platform view $id is not registered — " +
                            "the binding names no mounted view",
                    )
            slot.nativeGeometry = true
            if (slot.nativeWidth != width || slot.nativeHeight != height ||
                slot.nativeTransform != transform
            ) {
                slot.nativeWidth = width
                slot.nativeHeight = height
                slot.nativeTransform.set(transform)
            }
            applyNativeGeometry(slot)
            slot.view.drawNative(canvas)
        }
    private var pendingJson: String? = null
    private var appliedJson: String? = null
    private var orderedContent: OrderedContent? = null
    private val slotClip = Rect()
    private val inverse = Matrix()
    private val point = FloatArray(2)
    private var savedSplitting = true

    /**
     * Installs (or, with `null`, removes) the painter whose entries draw in
     * session order inside [container]'s `dispatchDraw`, interleaved with
     * the mounted platform views. Without one the slots stack above the GPU
     * band as always.
     *
     * While a painter is installed, motion-event splitting is off: with it
     * on, ViewGroup would route a second pointer's DOWN to the topmost
     * *child* under it — a slot that sits behind topmost self-drawn content
     * — splitting one logical gesture across two owners. The prior value is
     * restored when the painter is removed.
     */
    internal fun setOrderedContent(content: OrderedContent?) {
        if (content != null && orderedContent == null) {
            savedSplitting = container.isMotionEventSplittingEnabled
            container.isMotionEventSplittingEnabled = false
        } else if (content == null && orderedContent != null) {
            container.isMotionEventSplittingEnabled = savedSplitting
        }
        if (content == null && orderedContent != null) {
            for (slot in slots.values) {
                if (slot.nativeGeometry) {
                    slot.nativeGeometry = false
                    slot.view.animationMatrix = null
                    applySlot(slot)
                }
            }
        }
        orderedContent = content
        container.invalidate()
    }

    /**
     * The mounted slot of placement `id` when it is visible under host pixel
     * (`x`, `y`): the point mapped through the view's matrix into its local
     * bounds — `animationMatrix` makes the plain axis-aligned test wrong.
     * A declared placement with no mounted view is a session bug, never a
     * silent miss.
     */
    private fun slotAt(id: Long, x: Float, y: Float): View? {
        val view =
            slots[id]?.view
                ?: error("hydrolysis: ordered content names unmounted platform view $id")
        if (!view.isVisible) return null
        if (!view.matrix.invert(inverse)) return null
        point[0] = x - view.left
        point[1] = y - view.top
        inverse.mapPoints(point)
        val inside =
            point[0] >= 0 && point[0] < view.width && point[1] >= 0 && point[1] < view.height
        if (!inside) return null
        if (view.getClipBounds(slotClip)) {
            val clipped =
                point[0] >= slotClip.left &&
                    point[0] < slotClip.right &&
                    point[1] >= slotClip.top &&
                    point[1] < slotClip.bottom
            if (!clipped) return null
        }
        return view
    }

    /** Whether the platform view of placement `id` covers (`x`, `y`). */
    internal fun containsPlatformView(id: Long, x: Float, y: Float): Boolean =
        slotAt(id, x, y) != null

    /**
     * Whether a mounted child inside this container currently holds UI
     * focus. Read by the frame — reporting focus from the
     * `OnGlobalFocusChangeListener` would re-enter the session while it is
     * borrowed, since the listener fires synchronously inside
     * `requestFocus`/`removeView` on the host's own paths.
     */
    internal fun focusInside(): Boolean = container.findFocus() != null

    /** Registers the view factory for platform views of `kind`. */
    fun registerFactory(kind: String, factory: (Context) -> View) {
        factories[kind] = factory
        // A placement may already be waiting for this kind.
        pendingJson?.let { applyJson(it) }
    }

    /**
     * Pulls the session's placement set. Reached only through
     * [HydrolysisSession.requestPlacementPull], which runs it at native depth
     * zero: the pull is a session JNI call and must not nest in another.
     */
    internal fun publishIfPending() {
        val session = session ?: return
        val json =
            session.withNativePtr(NativeBridge::nativePlatformViewFrames.name) { ptr ->
                NativeBridge.nativePlatformViewFrames(ptr)
            } ?: return
        // The JNI side always serializes the current set, and onLayout pulls
        // it once per layout pass — re-applying an unchanged frame would
        // re-request layout inside the very pass that read it, forever.
        if (json == appliedJson) return
        appliedJson = json
        applyJson(json)
    }

    /** Mounted platform views, in `order`, grafted onto the a11y host node. */
    internal fun accessibilityChildren(): List<View> =
        slots.values.filter { it.view.isVisible }.map { it.view }

    /**
     * Whether a point in the container's coordinate space (== host view
     * pixels, the container fills the host at 0,0) falls on a mounted slot.
     * Hover dispatch uses this to keep real children on the normal dispatch
     * path and let the a11y provider own everything else.
     */
    internal fun coversPixel(x: Float, y: Float): Boolean {
        orderedContent?.let { content ->
            // The retained tree decides: the pixel is a platform hit only
            // when a platform view is drawn there and no interactive target
            // above it takes the point.
            val id = content.platformViewAt(x, y) ?: return false
            return slotAt(id, x, y) != null
        }
        val rect = Rect()
        for (slot in slots.values) {
            val view = slot.view
            if (!view.isVisible) continue
            view.getHitRect(rect)
            if (rect.contains(x.toInt(), y.toInt())) return true
        }
        return false
    }

    internal fun applyJson(json: String) {
        pendingJson = json
        val array = JSONArray(json)
        val wanted = ArrayList<Placement>(array.length())
        for (i in 0 until array.length()) {
            val entry = array.getJSONObject(i)
            val clip =
                entry.optJSONArray("clip")?.let { clipArray ->
                    floatArrayOf(
                        clipArray.optDouble(0, 0.0).toFloat(),
                        clipArray.optDouble(1, 0.0).toFloat(),
                        clipArray.optDouble(2, 0.0).toFloat(),
                        clipArray.optDouble(3, 0.0).toFloat(),
                    )
                }
            wanted.add(
                Placement(
                    id = entry.getLong("id"),
                    // The source is either a factory `kind` or a registered
                    // `instance` — flattened, so exactly one key is present.
                    kind = entry.optString("kind").takeIf { it.isNotEmpty() },
                    instance =
                        if (entry.has("instance")) {
                            entry.getLong("instance")
                        } else {
                            null
                        },
                    x = entry.optDouble("x", 0.0).toFloat(),
                    y = entry.optDouble("y", 0.0).toFloat(),
                    width = entry.optDouble("width", 0.0).toFloat(),
                    height = entry.optDouble("height", 0.0).toFloat(),
                    clip = clip,
                    order = entry.optLong("order", 0L),
                    visible = entry.optBoolean("visible", true),
                )
            )
        }
        wanted.sortBy { it.order }

        val stale = slots.keys - wanted.mapTo(HashSet()) { it.id }
        for (id in stale) {
            slots.remove(id)?.let { removeSlot(it) }
        }
        var changed = false
        for (placement in wanted) {
            val slot =
                slots.getOrPut(placement.id) {
                    changed = true
                    val slot = createSlot(placement)
                    // Attach before applySlot configures the generated
                    // FrameLayout.LayoutParams; orderSlots only reorders.
                    container.addView(slot.view)
                    slot
                }
            slot.placement = placement
            applySlot(slot)
        }
        if (changed || orderChanged(wanted)) orderSlots(wanted)
        container.requestLayout()
        container.invalidate()
    }

    private fun orderChanged(wanted: List<Placement>): Boolean {
        var i = 0
        for (placement in wanted) {
            val slot = slots[placement.id] ?: continue
            if (container.indexOfChild(slot.view) != i) return true
            i++
        }
        return container.childCount != i
    }

    private fun orderSlots(wanted: List<Placement>) {
        for ((index, placement) in wanted.withIndex()) {
            val slot = slots[placement.id] ?: continue
            if (container.indexOfChild(slot.view) != index) {
                container.removeView(slot.view)
                container.addView(slot.view, index)
            }
        }
    }

    /**
     * The touch-owning frame: filling the frame rect, `clipBounds` in the
     * slot's own coordinate space (the renderer's clip is already intersected
     * with the frame), and clickable so unhandled touches cannot fall through
     * to the WaterUI scene under the view.
     */
    /**
     * Drops `slot` — and frees an instance for remount by removing it from
     * the slot first. A View has exactly one parent; an instance that stayed
     * attached to a discarded slot could never mount again.
     */
    private fun removeSlot(slot: Slot) {
        slot.view.removeView(slot.mounted)
        container.removeView(slot.view)
    }

    /** Detaches every mounted instance view, on session unbind. */
    internal fun detachInstances() {
        for (slot in slots.values) {
            if (slot.placement.instance != null) {
                slot.view.removeView(slot.mounted)
            }
        }
    }

    /** `HydrolysisSession.unregisterPlatformViewInstance` under `id`. */
    internal fun onInstanceUnregistered(id: Long) {
        val holding = slots.keys.filter { slots[it]?.placement?.instance == id }
        for (slotId in holding) {
            slots.remove(slotId)?.let { removeSlot(it) }
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun createSlot(placement: Placement): Slot {
        val mounted =
            when {
                placement.instance != null -> {
                    val instanceId = placement.instance
                    val session =
                        session
                            ?: throw IllegalStateException(
                                "hydrolysis: a platform-view placement names " +
                                    "instance $instanceId but this registry has no session",
                            )
                    session.platformViewInstance(instanceId)?.also { instance ->
                        check(instance.parent == null) {
                            "hydrolysis: platform-view instance $instanceId is already " +
                                "mounted elsewhere — one View cannot have two parents"
                        }
                    }
                        ?: throw IllegalStateException(
                            "hydrolysis: platform-view instance $instanceId is not " +
                                "registered on the session",
                        )
                }
                placement.kind != null -> {
                    val factory =
                        factories[placement.kind]
                            ?: throw IllegalStateException(
                                "hydrolysis: no platform-view factory registered for " +
                                    "kind \"${placement.kind}\"; call " +
                                    "PlatformViewRegistry.registerFactory(kind) from the " +
                                    "application's host setup",
                            )
                    factory(context)
                }
                else ->
                    throw IllegalStateException(
                        "hydrolysis: a platform-view placement names neither " +
                            "a kind nor an instance",
                    )
            }
        val slotView =
            NativeSlot(context, mounted).apply {
                addView(
                    mounted,
                    FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT,
                    ),
                )
                // The slot is not itself semantic: the mounted view inside keeps
                // its own accessibility and the provider grafts it onto the host
                // node, so there is exactly one node per native child.
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                isClickable = true
            }
        return Slot(slotView, mounted)
    }

    private fun applySlot(slot: Slot) {
        if (slot.nativeGeometry) {
            // Native geometry owns the slot: the binding's local pixel size
            // and transform; no window-space margins, no double clip.
            applyNativeGeometry(slot)
            slot.view.visibility =
                if (slot.placement.visible) View.VISIBLE else View.INVISIBLE
            return
        }
        val placement = slot.placement
        val view = slot.view
        val density = context.resources.displayMetrics.density
        fun px(value: Float) = (value * density).toInt()

        val params = view.layoutParams as FrameLayout.LayoutParams
        val width = px(placement.width)
        val height = px(placement.height)
        val left = px(placement.x)
        val top = px(placement.y)
        if (params.width != width || params.height != height ||
            params.leftMargin != left || params.topMargin != top
        ) {
            params.width = width
            params.height = height
            params.leftMargin = left
            params.topMargin = top
            view.layoutParams = params
        }
        view.clipBounds =
            placement.clip?.let { clip ->
                Rect(
                    px(clip[0]) - left,
                    px(clip[1]) - top,
                    px(clip[2]) - left,
                    px(clip[3]) - top,
                )
            }
        view.visibility = if (placement.visible) View.VISIBLE else View.INVISIBLE
    }

    /**
     * Lays the slot out in the binding's pixel space. A translation-only
     * world matrix becomes the slot's real position, so `getBoundsOnScreen`
     * — which ignores `animationMatrix` — reports the drawn rect for
     * accessibility and focus; other transforms keep the matrix on the
     * view (Android's documented limit for native children, matching
     * Compose). Ancestor `RenderNode`s still own clip, alpha and effects,
     * and the placement's window clip stays on the slot for touch routing.
     */
    private fun applyNativeGeometry(slot: Slot) {
        val view = slot.view
        val width = slot.nativeWidth
        val height = slot.nativeHeight
        val values = FloatArray(9)
        slot.nativeTransform.getValues(values)
        val translationOnly =
            values[0] == 1f && values[1] == 0f && values[3] == 0f && values[4] == 1f &&
                values[6] == 0f && values[7] == 0f && values[8] == 1f
        val offsetX = if (translationOnly) values[2].toInt() else 0
        val offsetY = if (translationOnly) values[5].toInt() else 0
        val params = view.layoutParams as FrameLayout.LayoutParams
        if (params.width != width || params.height != height ||
            params.leftMargin != offsetX || params.topMargin != offsetY
        ) {
            params.width = width
            params.height = height
            params.leftMargin = offsetX
            params.topMargin = offsetY
            view.layoutParams = params
        }
        // NativeSlot's override compares and copies — no allocation here.
        view.animationMatrix = if (translationOnly) null else slot.nativeTransform
        view.clipBounds =
            slot.placement.clip?.let { clip ->
                val density = context.resources.displayMetrics.density
                Rect(
                    (clip[0] * density).toInt() - offsetX,
                    (clip[1] * density).toInt() - offsetY,
                    ((clip[0] + clip[2]) * density).toInt() - offsetX,
                    ((clip[1] + clip[3]) * density).toInt() - offsetY,
                )
            }
        if (view.isLayoutRequested || view.width != width ||
            view.height != height || view.left != offsetX || view.top != offsetY
        ) {
            view.measure(
                View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
            )
            view.layout(offsetX, offsetY, offsetX + width, offsetY + height)
        }
    }

    /** A slot whose mounted child can be recorded into a portal canvas. */
    private class NativeSlot(context: Context, val mounted: View) : FrameLayout(context) {
        /** The last transform applied through `animationMatrix` — kept to skip the no-change setter. */
        private val appliedMatrix = Matrix()

        override fun setAnimationMatrix(matrix: Matrix?) {
            if (matrix == appliedMatrix) return
            if (matrix == null) {
                appliedMatrix.reset()
            } else {
                appliedMatrix.set(matrix)
            }
            super.setAnimationMatrix(matrix)
        }

        /**
         * Draws the mounted child's own display list on the portal canvas —
         * `drawChild` so the child records under local `(0,0)`, the way its
         * real parent would draw it.
         */
        fun drawNative(canvas: Canvas) {
            drawChild(canvas, mounted, drawingTime)
        }
    }

    private class Slot(val view: NativeSlot, val mounted: View) {
        lateinit var placement: Placement
        /// A `BindPlatformView` owns this slot's geometry while true: the
        /// view draws inside its owner's `RenderNode`, so window-space
        /// margins, clip and size step aside for the binding's local size.
        var nativeGeometry = false
        var nativeWidth = 0
        var nativeHeight = 0
        val nativeTransform = Matrix()
    }

    private class Placement(
        val id: Long,
        val kind: String?,
        val instance: Long?,
        val x: Float,
        val y: Float,
        val width: Float,
        val height: Float,
        val clip: FloatArray?,
        val order: Long,
        val visible: Boolean,
    )
}
