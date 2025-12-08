package __BUNDLE_IDENTIFIER__

import android.os.Bundle
import android.system.Os
import android.util.Log
import androidx.activity.ComponentActivity
import dev.waterui.android.runtime.WaterUiRootView
import dev.waterui.android.runtime.bootstrapWaterUiRuntime
import dev.waterui.android.runtime.configureHotReloadDirectory
import java.lang.Runtime

class MainActivity : ComponentActivity() {
    companion object {
        private const val TAG = "WaterUI.MainActivity"

        init {
            // TODO: Setup env from waterui.env properties
            loadWaterUiLibraries()
            bootstrapWaterUiRuntime()
        }

        private fun loadWaterUiLibraries() {
            try {
                // waterui_app is the standardized name used by `water build android`
                loadLibraryGlobal("waterui_app")
                loadLibraryGlobal("waterui_android")
            } catch (error: UnsatisfiedLinkError) {
                throw RuntimeException("Failed to load WaterUI native libraries", error)
            }
        }

        @Suppress("DiscouragedPrivateApi")
        private fun loadLibraryGlobal(name: String) {
            val runtime = Runtime.getRuntime()
            try {
                val method = Runtime::class.java.getDeclaredMethod(
                    "loadLibrary0",
                    ClassLoader::class.java,
                    String::class.java,
                )
                method.isAccessible = true
                method.invoke(runtime, null, name)
            } catch (ignored: ReflectiveOperationException) {
                System.loadLibrary(name)
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Hot reload host/port are embedded at compile time via build.rs.
        // We only need to configure the directory for downloading dylibs.
        val hotReloadDisabled = intent?.getBooleanExtra("WATERUI_DISABLE_HOT_RELOAD", false) ?: false
        if (!hotReloadDisabled) {
            configureHotReloadDirectory(codeCacheDir.absolutePath)
        }

        val rootView = WaterUiRootView(this)
        setContentView(rootView)
        Log.i(TAG, "WATERUI_ROOT_READY")
    }
}
