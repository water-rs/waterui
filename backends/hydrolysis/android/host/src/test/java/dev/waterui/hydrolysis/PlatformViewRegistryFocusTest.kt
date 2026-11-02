package dev.waterui.hydrolysis

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.RenderNode
import android.view.MotionEvent
import android.view.View
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
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
 *
 * The ordered-content contract joins it here: an [OrderedContent] painter
 * draws its entries inside the container interleaved with the mounted
 * platform views in session order, and the topmost entry under the pointer
 * owns the gesture.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PlatformViewRegistryFocusTest {

    /** A host view that counts its frame requests and records touches. */
    private class RecordingHostView(activity: Activity) :
        HydrolysisHostView(activity, null) {
        var frames = 0
            private set

        val touches = mutableListOf<Int>()

        override fun requestFrame() {
            frames += 1
        }

        @SuppressLint("ClickableViewAccessibility")
        override fun onTouchEvent(event: MotionEvent): Boolean {
            touches += event.actionMasked
            return true
        }
    }

    /** A mounted platform view recording its draws and touches, tagged by kind. */
    private class RecordingView(
        context: Context,
        private val events: MutableList<String>,
        private val tag: String,
    ) : View(context) {
        override fun draw(canvas: Canvas) {
            super.draw(canvas)
            events += "$tag:draw"
        }

        @SuppressLint("ClickableViewAccessibility")
        override fun onTouchEvent(event: MotionEvent): Boolean {
            events += "$tag:touch:${event.actionMasked}"
            return true
        }
    }

    /**
     * A test [OrderedContent]: `Long` entries name platform placements,
     * anything else is a node run. Coverage is a lambda so tests re-point it.
     */
    private class FakeContent(
        private val draws: MutableList<String>,
        private val nativeHit: (Long, Float, Float) -> Boolean,
    ) : OrderedContent {
        val entries = mutableListOf<Any>()
        var covered: (Int, Float, Float) -> Boolean = { _, _, _ -> true }

        override val entryCount: Int
            get() = entries.size

        override fun isPlatformView(index: Int): Boolean = entries[index] is Long

        override fun platformViewId(index: Int): Long = entries[index] as Long

        var recorder: PlatformViewRecorder? = null

        override fun preparePlatformViews(recorder: PlatformViewRecorder) {
            this.recorder = recorder
        }

        override fun platformViewAt(x: Float, y: Float): Long? {
            for (index in entries.indices.reversed()) {
                val entry = entries[index]
                if (entry is Long) {
                    if (nativeHit(entry, x, y)) return entry
                } else if (covered(index, x, y)) {
                    return null
                }
            }
            return null
        }

        override fun drawBackground(canvas: Canvas) {
            draws += "background"
        }

        override fun drawEntry(canvas: Canvas, index: Int) {
            draws += "node$index"
        }

        override fun covers(index: Int, x: Float, y: Float): Boolean = covered(index, x, y)
    }

    private fun host(): Pair<RecordingHostView, PlatformViewRegistry> {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val hostView = RecordingHostView(activity)
        activity.setContentView(hostView)
        return hostView to hostView.platformViewRegistry
    }

    private fun mount(
        hostView: RecordingHostView,
        events: MutableList<String>,
        placement: String,
        kinds: Map<String, String> = mapOf("rec" to "v"),
    ) {
        val registry = hostView.platformViewRegistry
        for ((kind, tag) in kinds) {
            registry.registerFactory(kind) { RecordingView(it, events, tag) }
        }
        registry.applyJson(placement)
        hostView.measure(
            View.MeasureSpec.makeMeasureSpec(400, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(400, View.MeasureSpec.EXACTLY),
        )
        hostView.layout(0, 0, 400, 400)
    }

    private fun touch(action: Int, x: Float, y: Float): MotionEvent =
        MotionEvent.obtain(0L, 0L, action, x, y, 0)

    private fun multiTouch(action: Int, vararg points: Pair<Float, Float>): MotionEvent {
        val count = points.size
        val properties =
            Array(count) { index ->
                MotionEvent.PointerProperties().apply {
                    id = index
                    toolType = MotionEvent.TOOL_TYPE_FINGER
                }
            }
        val coords =
            Array(count) { index ->
                MotionEvent.PointerCoords().apply {
                    x = points[index].first
                    y = points[index].second
                }
            }
        return MotionEvent.obtain(0L, 0L, action, count, properties, coords, 0, 0, 1f, 1f, 0, 0, 0, 0)
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

    @Test
    fun `ordered entries draw in session order around the platform view`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, PLACEMENT)

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += "a"
        content.entries += 7L
        content.entries += "b"
        registry.setOrderedContent(content)

        registry.container.draw(Canvas())
        assertEquals(listOf("background", "node0", "v:draw", "node2"), events)
    }

    @Test
    fun `the topmost entry owns the gesture`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, PLACEMENT)

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += "node"
        content.entries += 7L
        registry.setOrderedContent(content)

        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_DOWN, 10f, 10f))
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_UP, 10f, 10f))
        assertEquals(listOf("v:touch:0", "v:touch:1"), events)
        assertTrue(hostView.touches.isEmpty())

        // Same point, reversed order: the node run is now topmost and the
        // touch goes to the session through the host, not to the slot.
        content.entries.clear()
        content.entries += 7L
        content.entries += "node"
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_DOWN, 10f, 10f))
        assertEquals(listOf(MotionEvent.ACTION_DOWN), hostView.touches)
        assertEquals(listOf("v:touch:0", "v:touch:1"), events)
    }

    @Test
    fun `the gesture owner is latched at action down`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, PLACEMENT)

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += 7L
        registry.setOrderedContent(content)

        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_DOWN, 10f, 10f))
        assertEquals(listOf("v:touch:0"), events)

        // Reorder mid-gesture: the platform entry is gone beneath a node run,
        // but the gesture stays with the view that accepted the DOWN.
        content.entries.clear()
        content.entries += 7L
        content.entries += "node"
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_MOVE, 10f, 10f))
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_UP, 10f, 10f))
        assertEquals(listOf("v:touch:0", "v:touch:2", "v:touch:1"), events)
        assertTrue(hostView.touches.isEmpty())
    }

    @Test
    fun `a clipped-away region of the platform view falls to the content`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, CLIPPED)

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += "node"
        content.entries += 7L
        registry.setOrderedContent(content)

        // (75, 50) is inside the frame but outside the clip: the slot does
        // not claim it and the node run beneath takes the gesture.
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_DOWN, 75f, 50f))
        assertEquals(listOf(MotionEvent.ACTION_DOWN), hostView.touches)
        assertTrue(events.isEmpty())

        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_UP, 75f, 50f))
        hostView.touches.clear()
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_DOWN, 25f, 50f))
        assertTrue(hostView.touches.isEmpty())
        assertEquals(listOf("v:touch:0"), events)
    }

    @Test
    fun `covers pixel follows the ordered top entry`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, PLACEMENT)

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += 7L
        registry.setOrderedContent(content)
        assertTrue(registry.coversPixel(10f, 10f))

        // A covering node run above the slot claims the hover instead.
        content.entries += "node"
        assertFalse(registry.coversPixel(10f, 10f))

        // No painter: the original slot hit-rect path.
        registry.setOrderedContent(null)
        assertTrue(registry.coversPixel(10f, 10f))
    }

    @Test
    fun `without a painter the children draw as usual`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, PLACEMENT)

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += "a"
        registry.setOrderedContent(content)
        registry.setOrderedContent(null)

        registry.container.draw(Canvas())
        assertEquals(listOf("v:draw"), events)
    }

    @Test
    fun `an ordered entry naming no mounted view fails loudly`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, PLACEMENT)

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += 99L
        registry.setOrderedContent(content)

        val failure =
            assertThrows(IllegalStateException::class.java) {
                registry.container.draw(Canvas())
            }
        assertTrue(failure.message.orEmpty().contains("platform view 99"))
    }

    @Test
    fun `motion event splitting is off while ordered content is installed`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        // Two slots side by side; the node run covers the right one only.
        mount(
            hostView,
            events,
            """[
                {"id":7,"kind":"left","x":0,"y":0,"width":100,"height":100,"order":0,"visible":true},
                {"id":8,"kind":"right","x":100,"y":0,"width":100,"height":100,"order":1,"visible":true}
            ]""",
            mapOf("left" to "l", "right" to "r"),
        )

        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += 7L
        content.entries += 8L
        content.entries += "node"
        content.covered = { _, x, _ -> x >= 100f }
        registry.setOrderedContent(content)
        assertFalse(registry.container.isMotionEventSplittingEnabled)

        // Pointer 0 lands on the left slot: it owns the gesture.
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_DOWN, 10f, 10f))
        // Pointer 1 lands where the covering node run is topmost — over the
        // right slot. With splitting on, ViewGroup would still hand it to
        // that slot; with it off the second pointer joins the first target
        // and slot 8 never sees the touch.
        registry.container.dispatchTouchEvent(
            multiTouch(
                MotionEvent.ACTION_POINTER_DOWN or (1 shl MotionEvent.ACTION_POINTER_INDEX_SHIFT),
                10f to 10f,
                150f to 10f,
            ),
        )
        assertEquals(
            listOf(
                "l:touch:0",
                "l:touch:${MotionEvent.ACTION_POINTER_DOWN}",
            ),
            events,
        )

        registry.setOrderedContent(null)
        assertTrue(registry.container.isMotionEventSplittingEnabled)
    }


    @Test
    fun `a bound platform view records its native child inside the portal`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(hostView, events, PLACEMENT)
        val content = FakeContent(events, registry::containsPlatformView)
        content.entries += 7L
        registry.setOrderedContent(content)

        // dispatchDraw installs the recorder: the painter's
        // preparePlatformViews call happens inside the container's draw.
        registry.container.draw(Canvas())
        val recorder = content.recorder
            ?: error("dispatchDraw never called preparePlatformViews")

        // The portal stands in for the painter's RenderNode: the recorder
        // records the mounted child's own list inside it.
        val portal = RenderNode("test-portal")
        val transform = Matrix().apply { setTranslate(40f, 50f) }
        portal.setPosition(0, 0, 30, 20)
        val canvas = portal.beginRecording(30, 20)
        val recordedCanvasHardware = canvas.isHardwareAccelerated
        recorder.drawLocal(7L, canvas, 30, 20, transform)
        portal.endRecording()

        assertTrue(recordedCanvasHardware)
        assertTrue(portal.hasDisplayList())
        val mounted = registry.container.getChildAt(0).let { it as android.widget.FrameLayout }
            .getChildAt(0)
        assertEquals(30, mounted.width)
        assertEquals(20, mounted.height)
        // dispatchDraw draws the slot once; drawLocal records the child's
        // own list into the portal — two "v:draw" events total.
        assertEquals(listOf("background", "v:draw", "v:draw"), events)
    }


    @Test
    fun `a clipped sibling platform view yields the gesture to the lower view`() {
        val (hostView, registry) = host()
        val events = mutableListOf<String>()
        mount(
            hostView,
            events,
            OVERLAPPING,
            mapOf("low" to "low", "high" to "high"),
        )
        // The retained tree clips view 8 away at this point while the
        // Android slots still overlap — the hit query chooses the lower.
        val content =
            FakeContent(events) { id, x, y ->
                id != 8L && registry.containsPlatformView(id, x, y)
            }
        content.entries += 7L
        content.entries += 8L
        registry.setOrderedContent(content)
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_DOWN, 10f, 10f))
        content.entries.clear()
        content.entries += 8L
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_MOVE, 12f, 12f))
        registry.container.dispatchTouchEvent(touch(MotionEvent.ACTION_UP, 12f, 12f))
        assertEquals(listOf("low:touch:0", "low:touch:2", "low:touch:1"), events)
        assertTrue(hostView.touches.isEmpty())
    }

    private companion object {
        const val OVERLAPPING =
            """[{"id":7,"kind":"low","x":0,"y":0,"width":100,"height":100,"order":0,"visible":true},{"id":8,"kind":"high","x":0,"y":0,"width":100,"height":100,"order":1,"visible":true}]"""

        /** One recording platform view, full host space at logical units. */
        const val PLACEMENT =
            """[{"id":7,"kind":"rec","x":0,"y":0,"width":100,"height":100,"order":0,"visible":true}]"""

        /** Same view clipped to its left half. */
        const val CLIPPED =
            """[{"id":7,"kind":"rec","x":0,"y":0,"width":100,"height":100,"clip":[0,0,50,100],"order":0,"visible":true}]"""
    }
}
