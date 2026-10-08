package dev.waterui.hydrolysis.preview

import android.app.Activity
import android.app.Instrumentation
import android.os.Bundle
import android.system.Os
import android.util.Log
import java.io.File

/**
 * The `water preview --platform android` entry point on-device.
 *
 * The CLI pushes the preview-mode launcher cdylib, the staged assets and a
 * JSON run config into a per-run directory under this host's private files,
 * then runs this instrumentation through `am instrument -w -r`. `-w` makes
 * `am` return only when the run finishes, so its exit is the completion
 * signal — there is no polling, and the render's PNG lands where the run
 * config says.
 *
 * Every path extra arrives relative to `filesDir` and is resolved here, so
 * the wire never carries an absolute private path the caller could be wrong
 * about.
 *
 * Any failure — a missing argument, a library that refuses to load, a panic
 * in the preview — finishes [Activity.RESULT_CANCELED] with the error in the
 * result bundle rather than leaving the `-w` wait hanging.
 */
class HydrolysisPreviewInstrumentation : Instrumentation() {
    private lateinit var arguments: Bundle

    override fun onCreate(arguments: Bundle) {
        super.onCreate(arguments)
        this.arguments = arguments
        start()
    }

    override fun onStart() {
        super.onStart()
        try {
            val filesDir = targetContext.filesDir
            // The native preview reads its run config, the staged assets
            // root, and its scratch directory from the process environment —
            // set all three before any library loads, so nothing reads an
            // unset slot.
            Os.setenv(
                "WATERUI_PREVIEW_RUN_CONFIG",
                inFiles(filesDir, arguments.requireString("runConfig")),
                true,
            )
            // Absent when the project staged no assets — the runtime's own
            // fallback applies then.
            arguments.getString("assetsRoot")?.let {
                Os.setenv("WATERUI_ASSETS_ROOT", inFiles(filesDir, it), true)
            }
            Os.setenv("WATER_CACHE_DIR", targetContext.cacheDir.absolutePath, true)
            PreviewBridge.run(
                arguments.requireString("libraries").split(':').map { inFiles(filesDir, it) },
                targetContext.applicationContext,
            )
            finish(Activity.RESULT_OK, Bundle())
        } catch (error: Throwable) {
            Log.e(TAG, "preview run failed", error)
            finish(
                Activity.RESULT_CANCELED,
                Bundle().apply { putString("error", error.stackTraceToString()) },
            )
        }
    }

    private fun inFiles(filesDir: File, name: String): String = File(filesDir, name).absolutePath

    private fun Bundle.requireString(name: String): String =
        getString(name)
            ?: throw IllegalArgumentException(
                "hydrolysis preview: missing required instrumentation argument `$name`",
            )

    private companion object {
        const val TAG = "HydrolysisPreview"
    }
}
