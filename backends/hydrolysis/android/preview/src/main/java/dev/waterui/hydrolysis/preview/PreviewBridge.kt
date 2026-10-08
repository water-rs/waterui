package dev.waterui.hydrolysis.preview

import android.annotation.SuppressLint
import android.content.Context

/**
 * JNI boundary to the preview-mode launcher cdylib.
 *
 * [SCHEMA] moves in lock-step with `PREVIEW_JNI_SCHEMA` in the hydrolysis
 * crate's `runner/android/preview.rs`: a library built against a different
 * schema is rejected at `nativeInit` rather than letting the host call entry
 * points that changed underneath it.
 */
internal object PreviewBridge {
    private const val SCHEMA = 1

    /**
     * Loads [libraries] in order — `libc++_shared.so` first when the CLI
     * staged it, the launcher cdylib last so its `JNI_OnLoad` registers the
     * preview — checks the schema, then runs the registered preview.
     */
    // The debug-only preview host exists to `System.load` the push-staged
    // payload; it ships in no release artifact.
    @SuppressLint("UnsafeDynamicallyLoadedCode")
    fun run(libraries: List<String>, context: Context, logLevel: String?) {
        for (library in libraries) {
            System.load(library)
        }
        val reported = nativeInit(SCHEMA, logLevel)
        check(reported == SCHEMA) {
            "hydrolysis preview JNI schema mismatch: the host speaks $SCHEMA " +
                "but the loaded library reported $reported"
        }
        nativeRunPreview(context)
    }

    @JvmStatic
    private external fun nativeInit(schema: Int, logLevel: String?): Int

    @JvmStatic
    private external fun nativeRunPreview(context: Context)
}
