package dev.waterui.hydrolysis

import android.content.Context
import android.content.ContextWrapper
import android.content.res.AssetManager
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.util.ReflectionHelpers

@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE)
class HydrolysisEnvironmentTest {
    private val context: Context = RuntimeEnvironment.getApplication()
    private val root get() = File(context.filesDir, "waterui_assets")

    private fun relaunch() {
        HydrolysisEnvironment::class.java.getDeclaredField("prepared").apply {
            isAccessible = true
            setBoolean(HydrolysisEnvironment, false)
        }
    }

    @Before
    fun reset() {
        relaunch()
        root.deleteRecursively()
    }

    @Test
    fun firstLaunchCopiesThePackagedTreeAndPreparesEnvironment() {
        HydrolysisEnvironment.prepare(context)
        assertEquals("version-2\n", File(root, "waterui-sync-stamp").readText())
        assertEquals("packaged payload\n", File(root, "nested/payload.txt").readText())
    }

    @Test
    fun relaunchWithTheSameStampSkipsCopying() {
        HydrolysisEnvironment.prepare(context)
        File(root, "nested/payload.txt").writeText("unchanged local file")
        relaunch()
        HydrolysisEnvironment.prepare(context)
        assertEquals("unchanged local file", File(root, "nested/payload.txt").readText())
    }

    @Test
    fun updateReplacesAnOlderTreeAndRemovesDeletedAssets() {
        root.mkdirs()
        File(root, "waterui-sync-stamp").writeText("version-1")
        File(root, "removed.txt").writeText("old asset")
        HydrolysisEnvironment.prepare(context)
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
            HydrolysisEnvironment.prepare(withoutAssets)
        }
        assertTrue(error.message!!.contains("waterui_assets/waterui-sync-stamp"))
        assertTrue(error.message!!.contains("missing or unreadable"))
        assertFalse(root.exists())
        // A failed preparation must not poison the once-per-process guard.
        HydrolysisEnvironment.prepare(context)
        assertTrue(File(root, "nested/payload.txt").exists())
    }
}
