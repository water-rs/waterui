package __BUNDLE_IDENTIFIER__

import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import dev.waterui.android.runtime.WaterUiRoot
import dev.waterui.android.runtime.bootstrapWaterUiRuntime
import java.lang.Runtime

class MainActivity : ComponentActivity() {
    companion object {
        private const val TAG = "WaterUI.MainActivity"

        init {
            loadWaterUiLibraries()
            bootstrapWaterUiRuntime("__CRATE_NAME_SANITIZED__")
        }

        private fun loadWaterUiLibraries() {
            try {
                loadLibraryGlobal("__CRATE_NAME_SANITIZED__")
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

        setContent {
            Log.i(TAG, "WATERUI_ROOT_READY")
            WaterUiRoot()
        }
    }
}
