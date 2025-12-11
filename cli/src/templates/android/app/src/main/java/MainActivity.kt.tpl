package __BUNDLE_IDENTIFIER__

import android.os.Bundle
import android.system.Os
import android.util.Log
import androidx.appcompat.app.AppCompatActivity
import dev.waterui.android.runtime.WaterUiRootView
import dev.waterui.android.runtime.bootstrapWaterUiRuntime
import java.lang.Runtime

class MainActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "WaterUI.MainActivity"
        private const val ENV_PREFIX = "waterui.env."

        init {
            setupEnvironmentFromProperties()
            loadWaterUiLibraries()
            bootstrapWaterUiRuntime()
        }

        /**
         * Read system properties with prefix "waterui.env." and set them as environment variables.
         *
         * The CLI sets these properties via `adb shell setprop waterui.env.<KEY> <VALUE>`
         * before launching the app. This allows passing environment variables to the native
         * Rust code since Android doesn't support direct environment variable passing.
         */
        @Suppress("PrivateApi")
        private fun setupEnvironmentFromProperties() {
            try {
                // Use reflection to access SystemProperties (hidden API)
                val systemProperties = Class.forName("android.os.SystemProperties")
                val getMethod = systemProperties.getMethod("get", String::class.java, String::class.java)

                // Known environment variables that might be set by the CLI
                val knownEnvVars = listOf(
                    "WATERUI_HOT_RELOAD_HOST",
                    "WATERUI_HOT_RELOAD_PORT",
                    "WATERUI_HOT_RELOAD_DIR",
                    "RUST_LOG",
                    "RUST_BACKTRACE"
                )

                for (envVar in knownEnvVars) {
                    val propKey = ENV_PREFIX + envVar
                    val value = getMethod.invoke(null, propKey, "") as String
                    if (value.isNotEmpty()) {
                        try {
                            Os.setenv(envVar, value, true)
                            Log.d(TAG, "Set environment variable $envVar from system property")
                        } catch (e: Exception) {
                            Log.w(TAG, "Failed to set environment variable $envVar: ${e.message}")
                        }
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to read system properties: ${e.message}")
            }
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

        val rootView = WaterUiRootView(this)
        setContentView(rootView)
        Log.i(TAG, "WATERUI_ROOT_READY")
    }
}
