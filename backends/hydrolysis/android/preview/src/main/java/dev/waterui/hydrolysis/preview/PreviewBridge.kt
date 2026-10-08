package dev.waterui.hydrolysis.preview

import android.annotation.SuppressLint
import android.content.Context

/**
 * Loads the push-staged launcher cdylib and hands it the run.
 *
 * [System.load] loads absolute paths — the cdylib lives inside this app's
 * private files, exactly where the CLI copied it — so the bridge loads the
 * payload directly rather than adding `lib/` to the app's native library
 * path, which `Context` exposes no API for.
 */
object PreviewBridge {
    /**
     * Incremented in lock-step with `PREVIEW_JNI_SCHEMA` on the launcher
     * side. History:
     *  - 1: `nativeInit(schema, logLevel)` + `nativeRunPreview(context)`;
     *  - 2: `nativeInit(schema)` — the cdylib checks the argument it is
     *    handed instead of trusting it, and the run's log level reads the
     *    usual `RUST_LOG` environment.
     */
    const val SCHEMA = 2

    /**
     * Load the staged libraries in order — `libc++_shared.so` first when the
     * CLI staged it — run the schema handshake, then the registered preview.
     */
    fun run(
        libraries: List<String>,
        context: Context,
    ) {
        for (library in libraries) {
            loadStagedLibrary(library)
        }
        val reported = nativeInit(SCHEMA)
        check(reported == SCHEMA) {
            "hydrolysis preview: the launcher cdylib speaks schema $reported but this host speaks $SCHEMA"
        }
        nativeRunPreview(context)
    }

    // The debug-only preview host exists to `System.load` the push-staged
    // payload; it ships in no release artifact.
    @SuppressLint("UnsafeDynamicallyLoadedCode")
    private fun loadStagedLibrary(path: String) = System.load(path)

    @JvmStatic
    private external fun nativeInit(schema: Int): Int

    @JvmStatic
    private external fun nativeRunPreview(context: Context)
}
