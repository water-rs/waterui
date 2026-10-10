package dev.waterui.hydrolysis.preview

import android.annotation.SuppressLint
import android.content.Context

/**
 * Loads the push-staged launcher cdylib and hands it render requests.
 *
 * [System.load] loads absolute paths — the cdylib lives inside this app's
 * private files, exactly where the CLI copied it — so the bridge loads the
 * payload directly rather than adding `lib/` to the app's native library
 * path, which `Context` exposes no API for.
 *
 * A loaded library cannot be swapped, so [initialize] runs exactly once per
 * process, when the instrumentation starts; [render] serves each render
 * request the [PreviewHostServer] accepts afterwards.
 */
object PreviewBridge {
    /**
     * Incremented in lock-step with `PREVIEW_JNI_SCHEMA` on the launcher
     * side.
     */
    const val SCHEMA = 1

    /**
     * Load the staged libraries in order — `libc++_shared.so` first when the
     * CLI staged it — and run the schema handshake. A launcher built against
     * a different schema throws out of `nativeInit`.
     */
    fun initialize(libraries: List<String>) {
        for (library in libraries) {
            loadStagedLibrary(library)
        }
        nativeInit(SCHEMA)
    }

    /**
     * Run the registered preview once, as the environment the current
     * request set (`WATERUI_PREVIEW_RUN_CONFIG`, `WATERUI_ASSETS_ROOT`)
     * describes.
     */
    fun render(context: Context) = nativeRunPreview(context)

    // The debug-only preview host exists to `System.load` the push-staged
    // payload; it ships in no release artifact.
    @SuppressLint("UnsafeDynamicallyLoadedCode")
    private fun loadStagedLibrary(path: String) = System.load(path)

    @JvmStatic
    private external fun nativeInit(schema: Int): Int

    @JvmStatic
    private external fun nativeRunPreview(context: Context)
}
