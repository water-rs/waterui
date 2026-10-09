package {{ ctx.android_package_name() }}

import android.content.Intent
import android.os.Bundle
import android.system.Os
import android.util.Log
import android.view.View
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import dev.waterui.hydrolysis.HydrolysisActivity
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.HydrolysisSession
{%- if ctx.hydrolysis_android_has_painter_band() %}
import {{ ctx.hydrolysis_android_painter_band_import() }}
{%- endif %}
import java.io.File

/**
 * The generated app entry: a [HydrolysisActivity] that loads this project's
 * Hydrolysis cdylib and mounts the painter's band beneath every host child.
 *
 * Everything the CLI passes to the app — environment variables, the bundled
 * asset tree, the cache directory — must be in place before
 * [HydrolysisActivity.onCreate] loads the native library, so it is all set
 * up ahead of `super.onCreate`.
 */
class MainActivity : HydrolysisActivity() {

    override val nativeLibraryName: String = "{{ ctx.hydrolysis_android_native_library_name() }}"

    override fun createContentView(session: HydrolysisSession): View {
        val host = HydrolysisHostView(this, session)
        {%- if ctx.hydrolysis_android_has_painter_band() %}
        // The painter band is the bottom-most child; platform-view overlays
        // and native embeddings draw above it.
        host.addView({{ ctx.hydrolysis_android_painter_band_class() }}(this, session), 0)
        {%- endif %}
        return host
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        // The launch screen the system showed from the tap on the icon stays
        // until this activity's first frame.
        installSplashScreen()

        // super.onCreate loads the native library and registers the app, so
        // every environment hand-off lands before it.
        val assetsRoot = syncBundledAssets()
        Os.setenv("WATERUI_ASSETS_ROOT", assetsRoot.absolutePath, true)
        // Rust code that wants a cache directory resolves WATER_CACHE_DIR
        // first; the intent extras below still override this default.
        Os.setenv("WATER_CACHE_DIR", cacheDir.absolutePath, true)
        setupEnvironmentFromIntent(intent)

        super.onCreate(savedInstanceState)
        Log.i(TAG, "WATERUI_ROOT_READY")
    }

    companion object {
        private const val TAG = "WaterUI.MainActivity"
        private const val ENV_PREFIX = "waterui.env."

        /**
         * Read intent extras with prefix "waterui.env." and set them as environment variables.
         *
         * The CLI passes these extras via:
         * `adb shell am start ... --es waterui.env.<KEY> <VALUE>`
         *
         * This runs before loading native libraries so Rust can read env vars at startup.
         */
        private fun setupEnvironmentFromIntent(intent: Intent?) {
            val extras = intent?.extras ?: return

            for (key in extras.keySet()) {
                if (!key.startsWith(ENV_PREFIX)) continue

                val envVar = key.removePrefix(ENV_PREFIX)
                val value = extras.getString(key) ?: continue

                // A dev-server URL may only ever redirect a debuggable
                // build; a release APK always renders its staged bundle.
                if (envVar == "WATERUI_DEV_URL" && !BuildConfig.DEBUG) continue

                try {
                    Os.setenv(envVar, value, true)
                    Log.d(TAG, "Set environment variable $envVar from intent extra")
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to set environment variable $envVar: ${e.message}")
                }
            }
        }

    }

    // syncBundledAssets and copyAssetTree are instance members: they read
    // `assets` and `filesDir`, which a companion object cannot see.

    private fun syncBundledAssets(): File {
        val assetRoot = File(filesDir, "waterui_assets")
        val stampAsset = "waterui_assets/.waterui-sync-stamp"
        val bundledStamp = try {
            assets.open(stampAsset).bufferedReader().use { it.readText() }
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
        copyAssetTree("waterui_assets", assetRoot)
        File(assetRoot, ".waterui-sync-stamp").writeText(bundledStamp)
        return assetRoot
    }

    private fun copyAssetTree(assetPath: String, dest: File) {
        val children = assets.list(assetPath)?.filter { it.isNotEmpty() }.orEmpty()
        if (children.isEmpty()) {
            dest.parentFile?.mkdirs()
            assets.open(assetPath).use { input ->
                dest.outputStream().use { output -> input.copyTo(output) }
            }
            return
        }

        dest.mkdirs()
        for (child in children) {
            copyAssetTree("$assetPath/$child", File(dest, child))
        }
    }
}
