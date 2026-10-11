package {{ ctx.android_package_name() }}

import android.content.Intent
import android.os.Bundle
import android.system.Os
import android.util.Log
import androidx.appcompat.app.AppCompatActivity
import androidx.activity.enableEdgeToEdge
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import dev.waterui.android.runtime.WaterUiRootView
import dev.waterui.android.runtime.installWaterUiProcessEnvironment
import java.lang.Runtime

class MainActivity : AppCompatActivity() {
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

        private fun loadWaterUiLibraries() {
            try {
                // waterui_app is the standardized name used by `water build android`
                // It contains both the app code and JNI bindings (pure Rust JNI approach)
                loadLibraryGlobal("waterui_app")
            } catch (error: UnsatisfiedLinkError) {
                throw RuntimeException("Failed to load WaterUI native library", error)
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

    private lateinit var androidRuntimeLease: AndroidRuntimeLease

    override fun onCreate(savedInstanceState: Bundle?) {
        // The launch screen the system showed from the tap on the icon stays
        // until this activity's first frame; the view tree is built
        // synchronously below, so that frame is the app itself.
        installSplashScreen()
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        // Bundled asset sync + the runtime's env defaults; the intent
        // extras below still win by overwriting them.
        installWaterUiProcessEnvironment(this)

        setupEnvironmentFromIntent(intent)
        loadWaterUiLibraries()
        val waterUiApplication = application as? WaterUiApplication
            ?: error("WaterUI requires WaterUiApplication")
        androidRuntimeLease = waterUiApplication.acquireRuntime(this)

        val rootView = WaterUiRootView(this)
        setContentView(rootView)
        Log.i(TAG, "WATERUI_ROOT_READY")
    }

    override fun onDestroy() {
        super.onDestroy()
        check(::androidRuntimeLease.isInitialized) { "Android runtime lease is not initialized" }
        androidRuntimeLease.close()
    }
}
