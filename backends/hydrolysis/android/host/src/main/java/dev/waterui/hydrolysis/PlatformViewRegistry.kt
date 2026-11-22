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
import org.json.JSONObject

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

            private fun drawOrdered(canvas: Canvas, content: OrderedContent) {
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

            /** Set while `super.dispatchDraw` runs: drawing is paint order. */
            private var drawingChildren = false

            override fun dispatchDraw(canvas: Canvas) {
                val content = orderedContent
                if (content != null) return drawOrdered(canvas, content)
                // Without a painter the children are the paint order — the
                // GPU band's views and bands interleave by child index — so
                // the touch reorder below must not reach them. ViewGroup asks
                // `getChildDrawingOrder` for both drawing and its reversed
                // touch dispatch, and the draw runs inside this call.
                drawingChildren = true
                try {
                    super.dispatchDraw(canvas)
                } finally {
                    drawingChildren = false
                }
            }

            override fun getChildDrawingOrder(childCount: Int, drawingPosition: Int): Int {
                if (drawingChildren) return drawingPosition
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
                val content = (orderedContent ?: orderedHitTest)
                    ?: return super.onInterceptTouchEvent(event)
                if (event.actionMasked == MotionEvent.ACTION_DOWN) {
                    // The retained tree answers who owns the point — alpha,
                    // transforms, clips and scroll already composed there;
                    // the slot's own matrix/clip is the visible-geometry check.
                    val id = content.platformViewAt(event.x, event.y)
                    nativeGesture =
                        id != null && !isSurface(id) && slotAt(id, event.x, event.y) != null
                    nativeTarget = if (nativeGesture) slots[id]?.view else null
                }
                return !nativeGesture || super.onInterceptTouchEvent(event)
            }

            @SuppressLint("ClickableViewAccessibility")
            override fun onTouchEvent(event: MotionEvent): Boolean {
                if ((orderedContent != null || orderedHitTest != null) && !nativeGesture) {
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

    init {
        // The decoder plane of the GPU video player belongs to the backend,
        // whichever painter draws the window.
        factories[VideoSurfaceHostView.KIND] = { context -> VideoSurfaceHostView(context) }
    }

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
    private var orderedHitTest: OrderedHitTest? = null
    private var splittingSuppressed = false
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
        val hadContent = orderedContent != null
        orderedContent = content
        if (content == null && hadContent) {
            for (slot in slots.values) {
                if (slot.nativeGeometry) {
                    slot.nativeGeometry = false
                    slot.view.animationMatrix = null
                    applySlot(slot)
                }
            }
        }
        syncSplitting()
        container.invalidate()
    }

    /**
     * The GPU band's ordered hit test: the gesture-ownership answers the
     * touch paths need when the surfaces draw outside this container — a
     * painter that draws entries here installs the richer [OrderedContent]
     * instead (water-rs/waterui#2225). Splitting stays off for the same
     * reason: children are not stacked in paint order there either.
     */
    internal fun setOrderedHitTest(hitTest: OrderedHitTest?) {
        orderedHitTest = hitTest
        syncSplitting()
    }

    /**
     * While an ordered answer is installed — a painter's content or the
     * GPU band's hit test — motion-event splitting is off: with it on,
     * ViewGroup would route a second pointer's DOWN to the topmost *child*
     * under it, a slot that can sit behind topmost self-drawn content,
     * splitting one logical gesture across two owners. The prior value is
     * restored when the last ordered answer is removed.
     */
    private fun syncSplitting() {
        val suppressed = orderedContent != null || orderedHitTest != null
        if (suppressed == splittingSuppressed) return
        splittingSuppressed = suppressed
        if (suppressed) {
            savedSplitting = container.isMotionEventSplittingEnabled
            container.isMotionEventSplittingEnabled = false
        } else {
            container.isMotionEventSplittingEnabled = savedSplitting
        }
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

    /** Whether placement `id` mounts a [PlatformSurface]: a view that only draws and takes no input. */
    private fun isSurface(id: Long): Boolean = slots[id]?.mounted is PlatformSurface

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
        (orderedContent ?: orderedHitTest)?.let { content ->
            // The retained tree decides: the pixel is a platform hit only
            // when a platform view is drawn there and no interactive target
            // above it takes the point.
            val id = content.platformViewAt(x, y) ?: return false
            return !isSurface(id) && slotAt(id, x, y) != null
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
                    stretches = stretches(entry),
                    stretchRegion = entry.optJSONArray("stretch_region")?.floats(4),
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

    /**
     * The overscroll stretches of a placement entry, outermost first: the
     * stretch filter's parameters the engine warps the surrounding content
     * with, for the slot's [OverscrollStretchEffect] to warp the view by.
     */
    private fun stretches(entry: JSONObject): List<OverscrollStretch> {
        val array = entry.optJSONArray("stretches") ?: return emptyList()
        return List(array.length()) { index ->
            val stretch = array.getJSONObject(index)
            OverscrollStretch(
                params = stretch.getJSONArray("params").floats(OverscrollStretch.PARAMS),
                strength = stretch.getDouble("strength").toFloat(),
            )
        }
    }

    private fun JSONArray.floats(count: Int): FloatArray {
        check(length() == count) { "hydrolysis: expected $count numbers, got ${length()}" }
        return FloatArray(count) { getDouble(it).toFloat() }
    }

    /**
     * Creates the [RegionBand] a GPU host inserts right after the platform
     * view whose placement id it is given — the View that shows the
     * self-drawn content painted above that view (water-rs/waterui#2225).
     * The id is the one the engine names the band by. `null` on hosts whose
     * painter draws that content in order itself: platform views keep
     * their contiguous child order.
     */
    var bandFactory: ((Long) -> RegionBand)? = null

    /** The live bands, keyed by the placement id of the view each sits above. */
    private val bands = mutableMapOf<Long, RegionBand>()

    /** The band above the platform view whose placement id is [view], while one is mounted. */
    internal fun band(view: Long): RegionBand? = bands[view]

    private fun orderChanged(wanted: List<Placement>): Boolean {
        val step = if (bandFactory != null) 2 else 1
        var i = 0
        for (placement in wanted) {
            val slot = slots[placement.id] ?: continue
            if (container.indexOfChild(slot.view) != i * step) return true
            i++
        }
        return container.childCount != i * step
    }

    private fun orderSlots(wanted: List<Placement>) {
        // Platform view `i` sits at index `2i` under a band factory, its
        // own band at `2i + 1` — a View drawing in child order, above the
        // platform views before it and beneath the ones after, which is
        // exactly where the content above that view's mark paints. The band
        // is named by the view's placement id, the id the engine marks the
        // view's layer with, so a view the engine plans no region for (one
        // clipped away entirely) keeps an empty band rather than shifting
        // every later region onto the wrong view (water-rs/waterui#2225).
        val factory = bandFactory
        val live = HashSet<Long>()
        var index = 0
        for (placement in wanted) {
            val slot = slots[placement.id] ?: continue
            if (container.indexOfChild(slot.view) != index) {
                container.removeView(slot.view)
                container.addView(slot.view, index)
            }
            index++
            if (factory != null) {
                live.add(placement.id)
                val band = bands.getOrPut(placement.id) { factory(placement.id) }
                if (container.indexOfChild(band) != index) {
                    container.removeView(band)
                    container.addView(band, index)
                }
                index++
            }
        }
        // Bands above views no longer ordered here are stale.
        val stale = bands.keys.filter { it !in live }
        for (view in stale) {
            bands.remove(view)?.let(container::removeView)
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
        (mounted as? PlatformSurface)?.placed(
            session
                ?: throw IllegalStateException(
                    "hydrolysis: a platform surface was placed on a registry with no session",
                ),
            placement.id,
        )
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

        if (placement.stretches.isNotEmpty()) {
            applyStretch(slot, density)
            view.visibility = if (placement.visible) View.VISIBLE else View.INVISIBLE
            return
        }
        endStretch(slot)

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
        view.clipBounds = placement.clip?.let { slotClip(it, density, left, top) }
        view.visibility = if (placement.visible) View.VISIBLE else View.INVISIBLE
    }

    /**
     * The slot-local `clipBounds` of a placement `clip` — window logical
     * `[x, y, width, height]` — for a slot whose origin sits at host pixel
     * (`left`, `top`).
     */
    private fun slotClip(clip: FloatArray, density: Float, left: Int, top: Int): Rect =
        Rect(
            (clip[0] * density).toInt() - left,
            (clip[1] * density).toInt() - top,
            ((clip[0] + clip[2]) * density).toInt() - left,
            ((clip[1] + clip[3]) * density).toInt() - top,
        )

    /**
     * Lays the slot out for the overscroll stretches its placement carries
     * and warps it by them. The engine stretches the self-drawn content
     * around the view in its own passes; the view is a real View it cannot
     * warp, so the slot draws through the same stretch as a RenderEffect
     * ([OverscrollStretchEffect]). The warp carries the view's content past
     * its frame, so the slot takes the placement's stretch region and the
     * mounted view keeps its frame size and window position inside it
     * ([StretchLayout]), clipped by the region as an unstretched slot is by
     * its clip. Touches keep the frame's unwarped geometry: a stretch shows
     * while a scroll gesture owns the pointer or a fling settles.
     */
    private fun applyStretch(slot: Slot, density: Float) {
        val placement = slot.placement
        val region =
            placement.stretchRegion
                ?: error("hydrolysis: platform view ${placement.id} is stretched without a region")
        val layout =
            StretchLayout.of(
                floatArrayOf(placement.x, placement.y, placement.width, placement.height),
                region,
                density,
            )
        val view = slot.view
        val params = view.layoutParams as FrameLayout.LayoutParams
        if (params.width != layout.regionWidth || params.height != layout.regionHeight ||
            params.leftMargin != layout.regionLeft || params.topMargin != layout.regionTop
        ) {
            params.width = layout.regionWidth
            params.height = layout.regionHeight
            params.leftMargin = layout.regionLeft
            params.topMargin = layout.regionTop
            view.layoutParams = params
        }
        view.clipBounds = null
        val mounted = slot.mounted.layoutParams as FrameLayout.LayoutParams
        if (mounted.width != layout.mountedWidth || mounted.height != layout.mountedHeight ||
            mounted.leftMargin != layout.mountedLeft || mounted.topMargin != layout.mountedTop
        ) {
            mounted.width = layout.mountedWidth
            mounted.height = layout.mountedHeight
            mounted.leftMargin = layout.mountedLeft
            mounted.topMargin = layout.mountedTop
            slot.mounted.layoutParams = mounted
        }
        val effect =
            slot.stretch ?: OverscrollStretchEffect(stretchSource).also { slot.stretch = it }
        effect.update(view, placement.stretches, density, layout.regionLeft, layout.regionTop)
    }

    /** Draws `slot` unstretched again, its mounted view filling it. */
    private fun endStretch(slot: Slot) {
        val effect = slot.stretch ?: return
        slot.stretch = null
        effect.clear(slot.view)
        val mounted = slot.mounted.layoutParams as FrameLayout.LayoutParams
        mounted.width = ViewGroup.LayoutParams.MATCH_PARENT
        mounted.height = ViewGroup.LayoutParams.MATCH_PARENT
        mounted.leftMargin = 0
        mounted.topMargin = 0
        slot.mounted.layoutParams = mounted
    }

    /** The AGSL source, read once when the first stretch shows. */
    private val stretchSource: String by lazy { OverscrollStretchShader.source() }

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
            slot.placement.clip?.let {
                slotClip(it, context.resources.displayMetrics.density, offsetX, offsetY)
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

        /** The overscroll stretch this slot draws through while its placement carries one. */
        var stretch: OverscrollStretchEffect? = null
    }

    private class Placement(
        val id: Long,
        val kind: String?,
        val instance: Long?,
        val x: Float,
        val y: Float,
        val width: Float,
        val height: Float,
        /** The clip cutting the frame, in window logical units: `[x, y, width, height]`. */
        val clip: FloatArray?,
        val order: Long,
        val visible: Boolean,
        /** The overscroll stretches warping the view, outermost first. */
        val stretches: List<OverscrollStretch>,
        /** The window region the warped view draws into: `[x, y, width, height]`. */
        val stretchRegion: FloatArray?,
    )
}
