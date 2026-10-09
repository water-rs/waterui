package dev.waterui.hydrolysis

import android.content.Context
import android.system.Os
import android.util.Log
import java.io.File

/**
 * Syncs the packaged `waterui_assets` tree and prepares the native environment
 * once per process, before any session starts. Intent environment overrides
 * must be applied after [prepare].
 */
object HydrolysisEnvironment {
    private var prepared = false

    /** Prepares the process environment once; later calls return at once. */
    @Synchronized
    fun prepare(context: Context) {
        if (prepared) return
        val appContext = context.applicationContext
        val assetsRoot = syncBundledAssets(appContext)
        Os.setenv("WATERUI_ASSETS_ROOT", assetsRoot.absolutePath, true)
        Os.setenv("WATER_CACHE_DIR", appContext.cacheDir.absolutePath, true)
        prepared = true
    }

    private fun syncBundledAssets(context: Context): File {
        val assetRoot = File(context.filesDir, "waterui_assets")
        val stampAsset = "waterui_assets/waterui-sync-stamp"
        val bundledStamp = try {
            context.assets.open(stampAsset).bufferedReader().use { it.readText() }
        } catch (e: Exception) {
            throw IllegalStateException(
                "hydrolysis: packaged asset '$stampAsset' is missing or unreadable — " +
                    "the build did not stage the waterui_assets tree",
                e,
            )
        }
        val localStamp = File(assetRoot, "waterui-sync-stamp")
            .takeIf { it.exists() }
            ?.readText()
        if (localStamp == bundledStamp) return assetRoot

        assetRoot.deleteRecursively()
        assetRoot.mkdirs()
        copyAssetTree(context, "waterui_assets", assetRoot)
        File(assetRoot, "waterui-sync-stamp").writeText(bundledStamp)
        Log.d("WaterUI.Environment", "Synced waterui_assets into $assetRoot")
        return assetRoot
    }

    private fun copyAssetTree(context: Context, assetPath: String, dest: File) {
        val children = context.assets.list(assetPath)?.filter { it.isNotEmpty() }.orEmpty()
        if (children.isEmpty()) {
            dest.parentFile?.mkdirs()
            context.assets.open(assetPath).use { input ->
                dest.outputStream().use { output -> input.copyTo(output) }
            }
            return
        }
        dest.mkdirs()
        for (child in children) {
            copyAssetTree(context, "$assetPath/$child", File(dest, child))
        }
    }
}
