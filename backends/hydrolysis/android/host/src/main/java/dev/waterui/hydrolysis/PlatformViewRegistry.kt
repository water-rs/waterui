package dev.waterui.hydrolysis

import android.content.Context
import android.view.View
import android.widget.FrameLayout
import org.json.JSONArray

/**
 * The embedded-native-view registry: a transparent [FrameLayout] laid out on
 * top of the GPU band inside the host view. The session publishes the whole
 * placement set as one JSON frame (`[{id,x,y,width,height,visible}]` in
 * host-view physical px); the registry mirrors it exactly — views for unknown
 * ids stay parked until a producer registers a factory for them.
 *
 * Producers (the platform-view embedding step of the plan) register through
 * [registerFactory]; without one for an id the placement still tracks — the
 * frame applies the moment a factory lands.
 */
class PlatformViewRegistry internal constructor(
    context: Context,
    private val session: HydrolysisSession?,
) {
    /** The overlay container the host view adds as its topmost child. */
    internal val container: FrameLayout =
        FrameLayout(context).apply {
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        }

    private val factories = HashMap<Long, (Context) -> View>()
    private val mounted = HashMap<Long, View>()
    private val placements = HashMap<Long, Placement>()

    /** Registers the view factory for a platform-view `id`. */
    fun registerFactory(id: Long, factory: (Context) -> View) {
        factories[id] = factory
    }

    internal fun publishIfPending() {
        val sessionPtr = session?.nativePtr ?: return
        val json = NativeBridge.nativePlatformViewFrames(sessionPtr) ?: return
        val next = HashMap<Long, Placement>()
        val array = JSONArray(json)
        for (i in 0 until array.length()) {
            val entry = array.getJSONObject(i)
            val placement =
                Placement(
                    x = entry.optDouble("x", 0.0).toFloat(),
                    y = entry.optDouble("y", 0.0).toFloat(),
                    width = entry.optDouble("width", 0.0).toFloat(),
                    height = entry.optDouble("height", 0.0).toFloat(),
                    visible = entry.optBoolean("visible", true),
                )
            next[entry.getLong("id")] = placement
        }
        placements.clear()
        placements.putAll(next)
        applyPlacements()
    }

    private fun applyPlacements() {
        val stale = mounted.keys - placements.keys
        for (id in stale) {
            mounted.remove(id)?.let(container::removeView)
        }
        for ((id, placement) in placements) {
            val factory = factories[id] ?: continue
            val view = mounted.getOrPut(id) {
                factory(container.context).also { container.addView(it) }
            }
            val params = view.layoutParams as FrameLayout.LayoutParams
            params.width = placement.width.toInt()
            params.height = placement.height.toInt()
            params.leftMargin = placement.x.toInt()
            params.topMargin = placement.y.toInt()
            view.layoutParams = params
            view.visibility = if (placement.visible) View.VISIBLE else View.GONE
        }
    }

    private class Placement(
        val x: Float,
        val y: Float,
        val width: Float,
        val height: Float,
        val visible: Boolean,
    )
}
