package dev.waterui.hydrolysis

import android.content.Context
import android.widget.FrameLayout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.internal.DoNotInstrument

/**
 * The deferred mount's contract against a captive sync worker and the
 * shadowed [NativeBridge]: the session — and the library load before it —
 * waits for the asset sync, never runs on the calling thread, and never
 * starts for an owner the destroy beat to it.
 */
@RunWith(RobolectricTestRunner::class)
@Config(
    shadows = [ShadowNativeBridge::class],
    instrumentedPackages = ["dev.waterui.hydrolysis"],
)
@DoNotInstrument
class DeferredMountTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    private val worker = CaptiveSyncWorker()

    @Before
    fun reset() {
        ShadowNativeBridge.reset()
        HydrolysisEnvironment.resetForTest()
        HydrolysisEnvironment.syncExecutor = worker
    }

    private fun mount(activity: ClosingActivity): FrameLayout =
        HydrolysisEmbedding.createView(
            activity,
            activity,
            activity,
            activity.onBackPressedDispatcher,
            "waterui_app",
            createContentView = { HydrolysisHostView(activity, it) },
            onCloseRequested = {},
        ) as FrameLayout

    @Test
    fun theSessionMountsOnlyAfterTheAssetSyncCompletes() {
        val activity = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        val container = mount(activity)
        activity.setContentView(container)

        // The window shows the empty host view while the copy sits on the
        // worker: no session starts on the calling thread.
        assertTrue(container.isAttachedToWindow)
        assertEquals(0, container.childCount)
        assertEquals(0, ShadowNativeBridge.sessions)

        worker.runOffMain()

        assertEquals(1, ShadowNativeBridge.sessions)
        assertEquals(1, container.childCount)
        assertTrue(container.getChildAt(0).isAttachedToWindow)
    }

    @Test
    fun theEnvironmentAppliesAfterPreparationBeforeTheLibraryLoads() {
        var deliver: ((() -> Unit) -> Unit)? = null
        var sessionsAtApply = -1
        var loadsAtApply = -1
        val activity = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        val container = HydrolysisEmbedding.createView(
            activity,
            activity,
            activity,
            activity.onBackPressedDispatcher,
            "waterui_app",
            prepareEnvironmentOverrides = { callback -> deliver = callback },
            createContentView = { HydrolysisHostView(activity, it) },
            onCloseRequested = {},
        ) as FrameLayout
        activity.setContentView(container)

        worker.runOffMain()

        // Preparation was invoked, but nothing applies — and nothing loads —
        // until the apply closure is delivered.
        assertNotNull(deliver)
        assertEquals(0, ShadowNativeBridge.sessions)
        assertEquals(0, ShadowNativeBridge.loads)

        deliver!! {
            sessionsAtApply = ShadowNativeBridge.sessions
            loadsAtApply = ShadowNativeBridge.loads
        }

        // apply ran before the library load, which ran before the mount.
        assertEquals(0, sessionsAtApply)
        assertEquals(0, loadsAtApply)
        assertEquals(1, ShadowNativeBridge.loads)
        assertEquals(1, ShadowNativeBridge.sessions)
        assertEquals(1, container.childCount)
    }

    @Test
    fun anOwnerDestroyedMidPreparationNeverAppliesOrLoads() {
        var deliver: ((() -> Unit) -> Unit)? = null
        var applied = false
        val controller = Robolectric.buildActivity(ClosingActivity::class.java)
        val activity = controller.setup().get()
        HydrolysisEmbedding.createView(
            activity,
            activity,
            activity,
            activity.onBackPressedDispatcher,
            "waterui_app",
            prepareEnvironmentOverrides = { callback -> deliver = callback },
            createContentView = { HydrolysisHostView(activity, it) },
            onCloseRequested = {},
        )

        worker.runOffMain()
        assertNotNull(deliver)

        controller.destroy()
        deliver!! { applied = true }

        assertFalse(applied)
        assertEquals(0, ShadowNativeBridge.loads)
        assertEquals(0, ShadowNativeBridge.sessions)
    }

    @Test
    fun aSecondMountInOneProcessAttachesAtOnce() {
        prepareOnce()
        val activity = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        val container = mount(activity)
        activity.setContentView(container)

        // The process was already prepared: no sync ran, the session
        // mounted inline.
        assertEquals(0, worker.pending)
        assertEquals(1, ShadowNativeBridge.sessions)
        assertEquals(1, container.childCount)
    }

    @Test
    fun aDestroyedOwnerNeverStartsASession() {
        val controller = Robolectric.buildActivity(ClosingActivity::class.java)
        val activity = controller.setup().get()
        val container = mount(activity)
        activity.setContentView(container)

        // The owner is gone before the worker finishes: no session may
        // start for it, and the host view stays empty.
        controller.destroy()
        worker.runOffMain()

        assertEquals(0, ShadowNativeBridge.sessions)
        assertEquals(0, container.childCount)

        // The sync did complete — a live owner mounts at once — so the
        // silence above is the destroy check, not a dropped continuation.
        val second = Robolectric.buildActivity(ClosingActivity::class.java).setup().get()
        val secondContainer = mount(second)
        second.setContentView(secondContainer)
        assertEquals(1, ShadowNativeBridge.sessions)
        assertEquals(1, secondContainer.childCount)
    }

    @Test
    fun aConfigurationChangeMidSyncMountsOnlyTheRecreatedActivity() {
        val controller = Robolectric.buildActivity(RecordingHydrolysisActivity::class.java).setup()
        val first = controller.get()

        // The activity is recreated while the copy is still on the worker:
        // both mounts queue behind the one sync.
        controller.recreate()
        val recreated = controller.get()
        assertEquals(1, worker.pending)
        worker.runOffMain()

        // The destroyed activity's mount is skipped; the recreated one owns
        // the process's single session.
        assertTrue(first.sessions.isEmpty())
        assertEquals(1, recreated.sessions.size)
        assertEquals(1, ShadowNativeBridge.sessions)
        assertTrue(recreated.sessions.single().hostView!!.isAttachedToWindow)
    }

    private fun prepareOnce() {
        HydrolysisEnvironment.prepare(context) {}
        worker.runOffMain()
    }
}
