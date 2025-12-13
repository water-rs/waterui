package __BUNDLE_IDENTIFIER__

import android.content.Intent
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

        @Volatile
        private var runtimeBootstrapped = false

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

                try {
                    Os.setenv(envVar, value, true)
                    Log.d(TAG, "Set environment variable $envVar from intent extra")
                } catch (e: Exception) {
                    Log.w(TAG, "Failed to set environment variable $envVar: ${e.message}")
                }
            }
        }

        /**
         * Read system properties with prefix "waterui.env." and set them as environment variables.
         *
         * Older CLI versions set these properties via `adb shell setprop waterui.env.<KEY> <VALUE>`
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

        @Synchronized
        private fun ensureRuntimeBootstrapped() {
            if (runtimeBootstrapped) return
            loadWaterUiLibraries()
            bootstrapWaterUiRuntime()
            runtimeBootstrapped = true
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

        // Initialize MediaPickerManager for this activity
        dev.waterui.android.runtime.MediaPickerManager.initialize(this)

        setupEnvironmentFromIntent(intent)
        setupEnvironmentFromProperties()
        ensureRuntimeBootstrapped()

        val rootView = WaterUiRootView(this)
        setContentView(rootView)
        Log.i(TAG, "WATERUI_ROOT_READY")
    }
}
