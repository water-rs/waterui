package dev.waterui.hydrolysis

import android.content.Context
import android.content.ContextWrapper
import android.content.res.AssetManager
import android.os.Looper
import java.io.File
import java.util.concurrent.Executor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.util.ReflectionHelpers

@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE)
class HydrolysisEnvironmentTest {
    private val context: Context = RuntimeEnvironment.getApplication()
    private val root get() = File(context.filesDir, "waterui_assets")

    /**
     * Runs a full [HydrolysisEnvironment.prepare] cycle synchronously: the
     * sync executes on the calling thread through the direct executor
     * [reset] installs, and the main-thread hand-off drains through the
     * looper.
     */
    private fun prepareNow(context: Context, onPrepared: () -> Unit = {}) {
        HydrolysisEnvironment.prepare(context, onPrepared)
        shadowOf(Looper.getMainLooper()).idle()
    }

    /** A fresh process: the once-per-process state drops; the synced tree stays. */
    private fun relaunch() {
        HydrolysisEnvironment.resetForTest()
    }

    @Before
    fun reset() {
        HydrolysisEnvironment.resetForTest()
        // These tests check what the sync produces, not its threading — the
        // direct executor runs it inline; the hand-off still crosses the
        // main looper, which `prepareNow` drains.
        HydrolysisEnvironment.syncExecutor = Executor { it.run() }
        root.deleteRecursively()
    }

    @Test
    fun firstLaunchCopiesThePackagedTreeAndPreparesEnvironment() {
        prepareNow(context)
        assertEquals("version-2\n", File(root, "waterui-sync-stamp").readText())
        assertEquals("packaged payload\n", File(root, "nested/payload.txt").readText())
    }

    @Test
    fun relaunchWithTheSameStampSkipsCopying() {
        prepareNow(context)
        File(root, "nested/payload.txt").writeText("unchanged local file")
        relaunch()
        prepareNow(context)
        assertEquals("unchanged local file", File(root, "nested/payload.txt").readText())
    }

    @Test
    fun updateReplacesAnOlderTreeAndRemovesDeletedAssets() {
        root.mkdirs()
        File(root, "waterui-sync-stamp").writeText("version-1")
        File(root, "removed.txt").writeText("old asset")
        prepareNow(context)
        assertEquals("version-2\n", File(root, "waterui-sync-stamp").readText())
        assertEquals("packaged payload\n", File(root, "nested/payload.txt").readText())
        assertFalse(File(root, "removed.txt").exists())
    }

    @Test
    fun missingStampFailsWithANamedError() {
        val withoutAssets = object : ContextWrapper(context) {
            private val emptyAssets = ReflectionHelpers.callConstructor(AssetManager::class.java)
            override fun getApplicationContext(): Context = this
            override fun getAssets(): AssetManager = emptyAssets
        }
        val error = assertThrows(IllegalStateException::class.java) {
            prepareNow(withoutAssets)
        }
        assertTrue(error.message!!.contains("waterui_assets/waterui-sync-stamp"))
        assertTrue(error.message!!.contains("missing or unreadable"))
        assertFalse(root.exists())
        // A failed preparation must not poison the once-per-process guard.
        prepareNow(context)
        assertTrue(File(root, "nested/payload.txt").exists())
    }

    @Test
    fun theCopyRunsOffTheMainLooperAndHandsBackOnIt() {
        val worker = CaptiveSyncWorker()
        HydrolysisEnvironment.syncExecutor = worker
        var preparedOnMain = false
        HydrolysisEnvironment.prepare(context) {
            preparedOnMain = Looper.myLooper() == Looper.getMainLooper()
        }

        // The copy went to the worker's queue: nothing reached filesDir on
        // this thread, and the continuation is still waiting.
        assertEquals(1, worker.pending)
        assertFalse(File(root, "waterui-sync-stamp").exists())

        // The worker runs the copy off the main thread; the result still
        // has to cross the main-thread hand-off.
        worker.runOffMain()
        assertTrue(preparedOnMain)
        assertEquals("version-2\n", File(root, "waterui-sync-stamp").readText())
        assertEquals("packaged payload\n", File(root, "nested/payload.txt").readText())
    }
}
