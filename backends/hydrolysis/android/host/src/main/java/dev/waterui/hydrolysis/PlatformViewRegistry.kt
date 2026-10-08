package dev.waterui.hydrolysis

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Rect
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
) {
    /** The overlay container the host view adds as its topmost child. */
    internal val container: FrameLayout =
        FrameLayout(context).apply {
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            clipChildren = true
        }

    private val factories = HashMap<String, (Context) -> View>()
    private val slots = LinkedHashMap<Long, Slot>()
    private var pendingJson: String? = null
    private var appliedJson: String? = null

    /** Registers the view factory for platform views of `kind`. */
    fun registerFactory(kind: String, factory: (Context) -> View) {
        factories[kind] = factory
        // A placement may already be waiting for this kind.
        pendingJson?.let { applyJson(it) }
    }

    /**
     * Pulls the session's placement set. Called when the host view lays out
     * and on `onNativePlatformViewsChanged`, so a fresh publish is applied
     * even before the next traversal.
     */
    internal fun publishIfPending() {
        val sessionPtr = session?.nativePtr ?: return
        val json = NativeBridge.nativePlatformViewFrames(sessionPtr) ?: return
        // The JNI side always serializes the current set, and onLayout pulls
        // it once per layout pass — re-applying an unchanged frame would
        // re-request layout inside the very pass that read it, forever.
        if (json == appliedJson) return
        appliedJson = json
        applyJson(json)
    }

    internal fun notifyChanged() = publishIfPending()

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
        val rect = Rect()
        for (slot in slots.values) {
            val view = slot.view
            if (!view.isVisible) continue
            view.getHitRect(rect)
            if (rect.contains(x.toInt(), y.toInt())) return true
        }
        return false
    }

    private fun applyJson(json: String) {
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
            FrameLayout(context).apply {
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

    private class Slot(val view: FrameLayout, val mounted: View) {
        lateinit var placement: Placement
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
