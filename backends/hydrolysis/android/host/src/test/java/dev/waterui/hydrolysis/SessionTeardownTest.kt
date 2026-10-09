package dev.waterui.hydrolysis

import android.app.Activity
import android.content.Context
import android.os.Looper
import android.view.View
import android.view.ViewGroup
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import java.time.Duration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.annotation.internal.DoNotInstrument

/**
 * The session's teardown contract against a shadowed [NativeBridge]: the
 * shadow stands in for the Rust runner and records every frame that
 * reaches it, and its `nativeDestroySession` requests a frame the way the
 * runner does while its state drops.
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    shadows = [ShadowNativeBridge::class, ShadowHydrolysisEnvironment::class],
    // The shadow can only replace `NativeBridge`'s natives on an
    // instrumented class.
    instrumentedPackages = ["dev.waterui.hydrolysis"],
)
@DoNotInstrument
class SessionTeardownTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    @Before
    fun resetNative() {
        ShadowNativeBridge.reset()
    }

    @Test
    fun aFrameRequestedDuringTeardownNeverReachesNative() {
        val session = HydrolysisSession(context, onCloseRequested = {})
        HydrolysisHostView(context, session)
        val looper = shadowOf(Looper.getMainLooper())

        // The pump works: a redraw on the live session reaches native.
        session.onNativeRequestRedraw()
        looper.idleFor(FRAME_WINDOW)
        assertEquals(1, ShadowNativeBridge.frames)

        ShadowNativeBridge.onDestroy = {
            session.onNativeRequestRedraw()
            // The request queued a vsync frame: teardown really posted one.
            assertNotEquals(Duration.ZERO, looper.nextScheduledTaskTime)
        }
        session.destroy()
        assertTrue(ShadowNativeBridge.destroyed)
        // A request after teardown posts nothing either.
        session.onNativeRequestRedraw()
        looper.idleFor(FRAME_WINDOW)

        assertEquals(1, ShadowNativeBridge.frames)
    }

    @Test
    fun teardownWaitsForTheAttachedHostViewToDetach() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val session = HydrolysisSession(activity, onCloseRequested = { activity.finish() })
        val host = HydrolysisHostView(activity, session)
        activity.setContentView(host)
        assertTrue(host.isAttachedToWindow)

        // `onDestroy` runs before the window is removed: the attached view
        // still reaches a live session.
        session.destroy()
        assertFalse(ShadowNativeBridge.destroyed)
        val visibilityCalls = ShadowNativeBridge.visibilities.size
        session.setVisible(false)
        assertEquals(visibilityCalls, ShadowNativeBridge.visibilities.size)

        (host.parent as ViewGroup).removeView(host)
        assertTrue(ShadowNativeBridge.destroyed)
        session.setVisible(false)
        assertEquals(visibilityCalls, ShadowNativeBridge.visibilities.size)
    }

    @Test
    fun aCloseThatTearsTheSessionDownRunsAfterTheFrame() {
        val activity = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        val session = HydrolysisSession(activity, onCloseRequested = { activity.finish() })
        val host = HydrolysisHostView(activity, session)
        activity.setContentView(host)
        activity.onFinish = {
            // An app's close handler: end the session, then drop the view.
            session.destroy()
            (host.parent as ViewGroup).removeView(host)
        }
        ShadowNativeBridge.onFrame = {
            session.onNativeCloseRequested()
            // The handler runs after the frame: the runner keeps using the
            // session after the request.
            assertTrue(host.isAttachedToWindow)
            assertFalse(ShadowNativeBridge.destroyed)
            // The next frame is queued behind the close: it must never
            // reach the session the close ends.
            WANTS_NEXT_FRAME
        }

        session.onNativeRequestRedraw()
        shadowOf(Looper.getMainLooper()).idleFor(FRAME_WINDOW)

        assertTrue(ShadowNativeBridge.destroyed)
        assertFalse(host.isAttachedToWindow)
        assertEquals(1, ShadowNativeBridge.frames)
    }

    @Test
    fun aTeardownRequestedInsideANativeCallWaitsForItToReturn() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val session = HydrolysisSession(activity, onCloseRequested = { activity.finish() })
        val host = HydrolysisHostView(activity, session)
        activity.setContentView(host)
        ShadowNativeBridge.onFrame = {
            // A callback that ends the session synchronously, mid-frame.
            session.destroy()
            (host.parent as ViewGroup).removeView(host)
            assertFalse(ShadowNativeBridge.destroyed)
            HAS_DEADLINE
        }

        session.onNativeRequestRedraw()
        shadowOf(Looper.getMainLooper()).idleFor(FRAME_WINDOW)

        assertTrue(ShadowNativeBridge.destroyed)
        assertEquals(1, ShadowNativeBridge.frames)
        assertEquals(0, ShadowNativeBridge.deadlineQueries)
    }

    @Test
    fun childrenDetachBeforeTheDeferredTeardown() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val session = HydrolysisSession(activity, onCloseRequested = { activity.finish() })
        val host = HydrolysisHostView(activity, session)
        var childReachedSession = false
        host.addView(
            object : View(activity) {
                override fun onDetachedFromWindow() {
                    session.setVisible(false)
                    childReachedSession = true
                    super.onDetachedFromWindow()
                }
            },
        )
        activity.setContentView(host)

        session.destroy()
        (host.parent as ViewGroup).removeView(host)

        assertTrue(childReachedSession)
        assertTrue(ShadowNativeBridge.destroyed)
    }

    /**
     * #2278 with a platform view: the window goes away while a mounted
     * instance holds focus. Unbinding frees the instance from its slot,
     * that removal clears child focus and requests a frame — on the live
     * session, before the deferred teardown — and the teardown must stop
     * the scheduler so the frame never reaches the freed session. The
     * registry keeps the host view after the unbind, so a request through
     * it later still has to post nothing.
     */
    @Test
    fun aFocusedPlatformViewReleasedByTheDeferredTeardownReachesNoFrame() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val session = HydrolysisSession(activity)
        val host = RecordingHostView(activity, session)
        activity.setContentView(host)
        val looper = shadowOf(Looper.getMainLooper())
        looper.idle()

        val instance =
            View(activity).apply {
                isFocusable = true
                isFocusableInTouchMode = true
            }
        ShadowNativeBridge.platformViewFrames =
            """[{"id":1,"instance":7,"x":0,"y":0,"width":10,"height":10,"order":0}]"""
        session.registerPlatformViewInstance(7, instance)
        assertTrue(instance.isAttachedToWindow)
        assertTrue(instance.requestFocus())
        assertTrue(host.platformViewRegistry.focusInside())
        looper.idleFor(FRAME_WINDOW)
        val framesBefore = ShadowNativeBridge.frames
        val requestsBefore = host.frameRequests

        var requestsAtTeardown = -1
        ShadowNativeBridge.onDestroy = {
            requestsAtTeardown = host.frameRequests
            // The instance is out of its slot before the session goes.
            assertNull(instance.parent)
        }
        // `onDestroy`: the view is still on its window, so the teardown waits.
        session.destroy()
        assertFalse(ShadowNativeBridge.destroyed)
        // The window goes: no `removeView` unfocuses the tree first, so the
        // instance still holds focus when the unbind removes it.
        activity.windowManager.removeViewImmediate(activity.window.decorView)

        assertTrue(ShadowNativeBridge.destroyed)
        assertTrue(
            "the slot removal requests a frame before the teardown",
            requestsAtTeardown > requestsBefore,
        )
        host.requestFrame()
        looper.idleFor(FRAME_WINDOW)
        assertEquals(framesBefore, ShadowNativeBridge.frames)
    }

    @Test
    fun aGenuineFinishReachesNoNativeCallAfterTeardown() {
        val controller = Robolectric.buildActivity(ClosingActivity::class.java).setup()
        val activity = controller.get()
        val host =
            HydrolysisEmbedding.createView(
                activity,
                activity,
                activity,
                activity.onBackPressedDispatcher,
                "waterui_app",
                onCloseRequested = {},
                createContentView = { session -> HydrolysisHostView(activity, session) },
            )
        activity.setContentView(host)
        assertTrue(host.isAttachedToWindow)

        // A real finish's order, inside `destroy`: `onDestroy` clears the
        // ViewModel store — destroy defers on the still-attached view —
        // the window's removal detaches it, `unbind` parks the live
        // session and tears it down, and the embedding's attach-state
        // listener runs last. Every shadow entry asserts the session is
        // live, so a call landing after that teardown — the listener's
        // old `setVisible` — fails the test inside `destroy`.
        controller.destroy()

        assertTrue(ShadowNativeBridge.destroyed)
        assertFalse(host.isAttachedToWindow)
    }

    @Test
    fun aClearedStoreParksTheSessionWhileItsViewStaysAttached() {
        val activity = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        val host =
            HydrolysisEmbedding.createView(
                activity,
                activity,
                activity,
                activity.onBackPressedDispatcher,
                "waterui_app",
                onCloseRequested = {},
                createContentView = { session -> HydrolysisHostView(activity, session) },
            )
        activity.setContentView(host)

        // A Compose navigation entry or a custom owner clears the store
        // with the view still attached: the session parks at once — it
        // stops pumping — and only waits on the view to end.
        activity.viewModelStore.clear()

        assertEquals(false, ShadowNativeBridge.visibilities.last())
        assertFalse(ShadowNativeBridge.destroyed)
        assertTrue(host.isAttachedToWindow)

        (host.parent as ViewGroup).removeView(host)
        assertTrue(ShadowNativeBridge.destroyed)
    }

    @Test
    fun aCreatedViewLifecycleParksBeforeAttachAndCannotRestartAClearedSession() {
        val activity = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        val owner = object : LifecycleOwner {
            val registry = LifecycleRegistry(this)
            override val lifecycle: Lifecycle get() = registry
        }
        owner.registry.currentState = Lifecycle.State.CREATED
        lateinit var session: HydrolysisSession
        val host = HydrolysisEmbedding.createView(
            activity, owner, activity, activity.onBackPressedDispatcher, "waterui_app",
            onCloseRequested = {},
            createContentView = {
                session = it
                HydrolysisHostView(activity, it)
            },
        )
        activity.setContentView(host)
        assertFalse(ShadowNativeBridge.visibilities.contains(true))
        owner.registry.currentState = Lifecycle.State.STARTED
        assertEquals(true, ShadowNativeBridge.visibilities.last())
        activity.viewModelStore.clear()
        val parkedCalls = ShadowNativeBridge.visibilities.size
        owner.registry.currentState = Lifecycle.State.CREATED
        owner.registry.currentState = Lifecycle.State.STARTED
        assertEquals(parkedCalls, ShadowNativeBridge.visibilities.size)
        assertEquals(false, ShadowNativeBridge.visibilities.last())
        val error = assertThrows(IllegalStateException::class.java) {
            session.bind(HydrolysisHostView(activity, session))
        }
        assertTrue(error.message!!.contains("bind reached a closing HydrolysisSession"))
        (host.parent as ViewGroup).removeView(host)
        assertTrue(ShadowNativeBridge.destroyed)
        assertThrows(IllegalStateException::class.java) { session.bind(host as HydrolysisHostView) }
    }

    @Test
    fun aCloseBetweenMountsWaitsForTheNextMount() {
        val activity = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        lateinit var session: HydrolysisSession
        var firstCloses = 0
        var nextCloses = 0
        fun mount(onClose: () -> Unit): View = HydrolysisEmbedding.createView(
            activity, activity, activity, activity.onBackPressedDispatcher, "waterui_app",
            createContentView = {
                session = it
                HydrolysisHostView(activity, it)
            },
            onCloseRequested = onClose,
        )
        val first = mount { firstCloses++ }
        activity.setContentView(first)
        (first.parent as ViewGroup).removeView(first)

        // The detached mount's handler is gone: the close waits.
        session.onNativeCloseRequested()
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(0, firstCloses)

        val retained = session
        val next = mount { nextCloses++ }
        assertTrue(retained === session)
        assertEquals(0, nextCloses)
        // The next mount's view attaching delivers the waiting close once.
        activity.setContentView(next)
        assertEquals(1, nextCloses)

        session.onNativeCloseRequested()
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(0, firstCloses)
        assertEquals(2, nextCloses)
        activity.viewModelStore.clear()
        (next.parent as ViewGroup).removeView(next)
    }

    @Test
    fun aRecreatedActivityKeepsItsSessionAndRebindsIt() {
        val controller = Robolectric.buildActivity(RecordingHydrolysisActivity::class.java).setup()
        val first = controller.get()
        val session = first.sessions.single()
        session.onNativeBackAvailable(true)
        assertTrue(first.onBackPressedDispatcher.hasEnabledCallbacks())

        controller.recreate()
        val recreated = controller.get()

        assertTrue(recreated !== first)
        // The ViewModel carried the one live session across the change.
        assertTrue(recreated.sessions.single() === session)
        assertFalse(ShadowNativeBridge.destroyed)
        // The new activity's host view is the bound one.
        val host = recreated.window.decorView.findViewById<ViewGroup>(android.R.id.content)
            .getChildAt(0)
        assertTrue(host.isAttachedToWindow)
        assertTrue(session.hostView === host)
        // The retained back answer re-enabled the new activity's callback,
        // and back reaches the session through it.
        assertTrue(recreated.onBackPressedDispatcher.hasEnabledCallbacks())
        assertFalse(first.onBackPressedDispatcher.hasEnabledCallbacks())
        recreated.onBackPressedDispatcher.onBackPressed()
        assertEquals(listOf(NativeBridge.BACK_INVOKED), ShadowNativeBridge.backEvents)

        controller.destroy()
        assertTrue(ShadowNativeBridge.destroyed)
    }

    @Test
    fun aVisibilityCallAfterDestroyIsIgnored() {
        val session = HydrolysisSession(context, onCloseRequested = {})
        session.destroy()

        val visibilityCalls = ShadowNativeBridge.visibilities.size
        session.setVisible(true)
        assertEquals(visibilityCalls, ShadowNativeBridge.visibilities.size)
    }

    private companion object {
        /** Long enough for Robolectric's Choreographer to run a posted frame. */
        val FRAME_WINDOW: Duration = Duration.ofSeconds(1)

        /** `nativeOnFrame`'s outcome bits: more work, a deadline frame due. */
        const val WANTS_NEXT_FRAME = 1L
        const val HAS_DEADLINE = 4L
    }
}

/** A host view that counts its frame requests and still schedules them. */
class RecordingHostView(context: Context, session: HydrolysisSession) :
    HydrolysisHostView(context, session) {
    var frameRequests = 0
        private set

    override fun requestFrame() {
        frameRequests += 1
        super.requestFrame()
    }
}

/** The standalone host over the shadowed library, recording its sessions. */
class RecordingHydrolysisActivity : HydrolysisActivity() {
    val sessions = mutableListOf<HydrolysisSession>()

    override val nativeLibraryName: String = "waterui_app"

    override fun createContentView(session: HydrolysisSession): View {
        sessions += session
        return HydrolysisHostView(this, session)
    }
}

/** An activity whose `finish` runs the test's close handler synchronously. */
class ClosingActivity : androidx.activity.ComponentActivity() {
    var onFinish: () -> Unit = {}

    override fun finish() {
        onFinish()
        super.finish()
    }
}

/**
 * The Rust runner's stand-in: a fake session pointer and a frame count.
 * Every entry asserts the session is still live, as the runner's borrow of
 * it would require.
 */
@Implements(NativeBridge::class, isInAndroidSdk = false)
@DoNotInstrument
class ShadowNativeBridge {
    companion object {
        private const val SESSION_PTR = 0x5e55_1011L

        var frames = 0
            private set

        var destroyed = false
            private set

        var deadlineQueries = 0
            private set

        /** Every `nativeSetVisible` call, in order. */
        val visibilities = mutableListOf<Boolean>()

        /** Every `nativeBackEvent` phase, in order. */
        val backEvents = mutableListOf<Int>()

        /** Runs inside `nativeDestroySession`, as the runner's drop does. */
        var onDestroy: () -> Unit = {}

        /** Runs inside `nativeOnFrame`; its result is the frame's outcome. */
        var onFrame: () -> Long = { 0L }

        /** What `nativePlatformViewFrames` publishes; none by default. */
        var platformViewFrames: String? = null
        /** The accessibility tree JSON `nativeAccessibilityTree` serves. */
        var treeJson: String? = null

        fun reset() {
            frames = 0
            destroyed = false
            deadlineQueries = 0
            visibilities.clear()
            backEvents.clear()
            onDestroy = {}
            onFrame = { 0L }
            platformViewFrames = null
            treeJson = null
        }

        private fun assertLive(sessionPtr: Long) {
            assertEquals(SESSION_PTR, sessionPtr)
            assertFalse("a native call reached a destroyed session", destroyed)
        }

        @JvmStatic
        @Implementation
        fun nativeInit(
            schema: Int,
            @Suppress("UNUSED_PARAMETER") logLevel: String?,
        ): Int = schema

        @JvmStatic
        @Implementation
        fun nativeUiThreadServices(): Long = 0x1111_2eadL

        @JvmStatic
        @Implementation
        fun nativeCreateSession(
            @Suppress("UNUSED_PARAMETER") session: HydrolysisSession,
            @Suppress("UNUSED_PARAMETER") context: Context,
            @Suppress("UNUSED_PARAMETER") uiThreadServices: Long,
        ): Long = SESSION_PTR

        @JvmStatic
        @Implementation
        fun nativeDestroySession(sessionPtr: Long) {
            assertLive(sessionPtr)
            onDestroy()
            destroyed = true
        }

        @JvmStatic
        @Implementation
        fun nativeOnFrame(sessionPtr: Long, @Suppress("UNUSED_PARAMETER") vsyncNanos: Long): Long {
            assertLive(sessionPtr)
            frames += 1
            return onFrame()
        }

        @JvmStatic
        @Implementation
        fun nativeFrameDeadlineInNanos(sessionPtr: Long): Long {
            assertLive(sessionPtr)
            deadlineQueries += 1
            return 0L
        }

        @JvmStatic
        @Implementation
        fun nativeSetVisible(sessionPtr: Long, visible: Boolean) {
            assertLive(sessionPtr)
            visibilities += visible
        }

        @JvmStatic
        @Implementation
        fun nativeBackEvent(
            sessionPtr: Long,
            phase: Int,
            @Suppress("UNUSED_PARAMETER") edge: Int,
            @Suppress("UNUSED_PARAMETER") progress: Double,
        ) {
            assertLive(sessionPtr)
            backEvents += phase
        }

        @JvmStatic
        @Implementation
        fun nativePlatformViewFrames(sessionPtr: Long): String? {
            assertLive(sessionPtr)
            return platformViewFrames
        }

        @JvmStatic
        @Implementation
        fun nativeSetHighRefresh(sessionPtr: Long, @Suppress("UNUSED_PARAMETER") active: Boolean) {
            assertLive(sessionPtr)
        }

        @JvmStatic
        @Implementation
        fun nativeAccessibilityTree(sessionPtr: Long): String? {
            assertLive(sessionPtr)
            return treeJson
        }
    }
}

/**
 * Isolates the session tests from process environment preparation. Real
 * asset synchronization is covered by HydrolysisEnvironmentTest without
 * this shadow.
 */
@Implements(HydrolysisEnvironment::class, isInAndroidSdk = false)
@DoNotInstrument
class ShadowHydrolysisEnvironment {
    @Implementation
    fun prepare(@Suppress("UNUSED_PARAMETER") context: Context) {}
}
