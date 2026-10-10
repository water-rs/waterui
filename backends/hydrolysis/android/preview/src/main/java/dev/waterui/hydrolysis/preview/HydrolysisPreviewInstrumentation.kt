package dev.waterui.hydrolysis.preview

import android.app.Activity
import android.app.Instrumentation
import android.os.Bundle
import android.system.Os
import android.util.Log
import java.io.File

/**
 * The `water preview --platform android` entry point on-device — the
 * preview host process itself.
 *
 * The CLI pushes the preview-mode launcher cdylib and the staged assets
 * into one fixed directory under this host's private files, then starts
 * this instrumentation through `am instrument` (no `-w`: the process is
 * meant to stay). Starting it force-stops the package, so a start is also
 * the restart the CLI issues when the payload's stamp changes — a loaded
 * library cannot be swapped, and an unchanged payload reuses the live
 * process instead.
 *
 * On start the instrumentation loads the staged libraries, then serves
 * render requests on a [LocalServerSocket][android.net.LocalServerSocket]
 * the CLI reaches through `adb forward`; each request re-points the run
 * config and assets-root environment slots and runs the registered preview
 * once — the same render a one-shot run produced. The process stays alive
 * between runs until the package is force-stopped or replaced.
 *
 * Every path arrives relative to `filesDir` and is resolved here, so the
 * wire never carries an absolute private path the caller could be wrong
 * about.
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
            Os.setenv("WATER_CACHE_DIR", targetContext.cacheDir.absolutePath, true)
            PreviewBridge.initialize(
                arguments.requireString("libraries").split(':').map { inFiles(filesDir, it) }
            )
            PreviewHostServer(
                context = targetContext.applicationContext,
                filesDir = filesDir,
                stamp = arguments.requireString("payloadStamp"),
            ).serve()
        } catch (error: Throwable) {
            // The host serves until it dies, so reaching this catch means
            // the process never came up — the next run's probe finds no
            // host and starts a fresh one, which reads this in logcat.
            Log.e(TAG, "preview host failed to start", error)
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