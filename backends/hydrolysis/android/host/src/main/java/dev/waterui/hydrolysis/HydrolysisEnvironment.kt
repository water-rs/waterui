package dev.waterui.hydrolysis

import android.content.Context
import android.system.Os
import android.util.Log
import java.io.File

/**
 * The process-level environment a Hydrolysis session expects: the bundled
 * `waterui_assets` tree synced to `filesDir` and the `WATERUI_ASSETS_ROOT`
 * and `WATER_CACHE_DIR` variables the native side reads.
 *
 * The contract is the CLI's: `water build`/`water run` stages the asset tree
 * named `waterui_assets` into the packaged `assets`, with a
 * `.waterui-sync-stamp` file recording the staged content's stamp — the sync
 * compares that stamp so an unchanged tree is never recopied.
 *
 * [prepare] runs once per process. Callers that hand extra environment
 * variables to the native library — the generated `MainActivity`'s
 * `waterui.env.*` intent extras — set them after [prepare], so their values
 * still override these defaults.
 */
object HydrolysisEnvironment {

    private const val TAG = "WaterUI.Environment"

    private var prepared = false

    /** Prepares the process environment once; later calls return at once. */
    @Synchronized
    fun prepare(context: Context) {
        if (prepared) return
        val appContext = context.applicationContext
        val assetsRoot = syncBundledAssets(appContext)
        Os.setenv("WATERUI_ASSETS_ROOT", assetsRoot.absolutePath, true)
        // Rust code that wants a cache directory resolves WATER_CACHE_DIR
        // first; the intent extras the generated MainActivity applies after
        // this still override the default.
        Os.setenv("WATER_CACHE_DIR", appContext.cacheDir.absolutePath, true)
        prepared = true
    }

    private fun syncBundledAssets(context: Context): File {
        val assetRoot = File(context.filesDir, "waterui_assets")
        val stampAsset = "waterui_assets/.waterui-sync-stamp"
        val bundledStamp = try {
            context.assets.open(stampAsset).bufferedReader().use { it.readText() }
        } catch (_: Exception) {
            assetRoot.mkdirs()
            return assetRoot
        }

        val localStamp = File(assetRoot, ".waterui-sync-stamp")
            .takeIf { it.exists() }
            ?.readText()
        if (localStamp == bundledStamp) {
            return assetRoot
        }

        assetRoot.deleteRecursively()
        assetRoot.mkdirs()
        copyAssetTree(context, "waterui_assets", assetRoot)
        File(assetRoot, ".waterui-sync-stamp").writeText(bundledStamp)
        Log.d(TAG, "Synced waterui_assets into $assetRoot")
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
