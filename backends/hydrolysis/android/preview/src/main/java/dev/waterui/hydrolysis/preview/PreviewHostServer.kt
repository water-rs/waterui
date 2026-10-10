package dev.waterui.hydrolysis.preview

import android.content.Context
import android.net.LocalServerSocket
import android.net.LocalSocket
import android.system.Os
import android.util.Log
import java.io.ByteArrayOutputStream
import java.io.EOFException
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import org.json.JSONException
import org.json.JSONObject

/**
 * The long-lived half of `water preview --platform android`: one process
 * that holds the loaded payload and serves render requests, so a warm run
 * skips the zygote fork, the `System.load` relocation, the GPU setup and
 * the font scan a fresh instrumentation pays every time.
 *
 * The CLI reaches this server through `adb forward` onto the abstract
 * socket [SOCKET_NAME]. Every connection carries one render, exchanged as
 * single-line JSON objects:
 *
 * - the host greets first — `{"type":"hello","schema":…,"stamp":…}` —
 *   naming the payload stamp it loaded, so the CLI can tell a live host
 *   that carries its payload from one that must be replaced;
 * - the CLI then sends `{"type":"render","run_config":…,"assets_root":…}`,
 *   both paths relative to the app's `filesDir` exactly as the
 *   instrumentation extras carried them;
 * - the host answers `{"type":"rendered"}` or
 *   `{"type":"failed","error":…}` and closes the connection.
 *
 * The schema must match `HOST_PROTOCOL_SCHEMA` on the CLI side; a change to
 * the message shapes bumps it.
 */
class PreviewHostServer(
    private val context: Context,
    private val filesDir: File,
    private val stamp: String,
) {
    /**
     * Accept and serve connections until the process dies. Once the socket
     * is bound it logs [SERVING] with the stamp — the readiness signal the
     * CLI waits on with `logcat -m 1`. It is reached only after the payload
     * libraries loaded, because the instrumentation starts this server
     * after [PreviewBridge.initialize].
     */
    fun serve(): Nothing {
        val server = LocalServerSocket(SOCKET_NAME)
        try {
            Log.i(TAG, "$SERVING: stamp $stamp")
            while (true) {
                exchange(server.accept())
            }
        } finally {
            server.close()
        }
    }

    private fun exchange(socket: LocalSocket) {
        try {
            // The greeting is written first, so a connect answers the CLI's
            // liveness and identity probe; the request then has this long
            // to arrive before the connection is abandoned.
            socket.soTimeout = REQUEST_DEADLINE_MS
            writeLine(socket.outputStream, hello().toString())
            val request = RenderRequest.parse(readLine(socket.inputStream))
            Os.setenv(
                "WATERUI_PREVIEW_RUN_CONFIG",
                inFiles(request.runConfig),
                true,
            )
            Os.setenv("WATERUI_ASSETS_ROOT", inFiles(request.assetsRoot), true)
            PreviewBridge.render(context)
            writeLine(socket.outputStream, RENDERED)
        } catch (error: Throwable) {
            Log.e(TAG, "preview request failed", error)
            // The reply is best-effort: a peer that went away takes the
            // write down with it, which still closes the connection.
            runCatching {
                writeLine(
                    socket.outputStream,
                    JSONObject()
                        .put("type", "failed")
                        .put("error", error.stackTraceToString())
                        .toString(),
                )
            }
        } finally {
            socket.close()
        }
    }

    private fun hello(): JSONObject =
        JSONObject()
            .put("type", "hello")
            .put("schema", PROTOCOL_SCHEMA)
            .put("stamp", stamp)

    private fun inFiles(name: String): String = File(filesDir, name).absolutePath

    private data class RenderRequest(
        val runConfig: String,
        val assetsRoot: String,
    ) {
        companion object {
            fun parse(line: String): RenderRequest {
                val message =
                    try {
                        JSONObject(line)
                    } catch (error: JSONException) {
                        throw IllegalArgumentException(
                            "hydrolysis preview: malformed request `$line`",
                            error,
                        )
                    }
                require(message.optString("type") == "render") {
                    "hydrolysis preview: expected a `render` request, got `$line`"
                }
                return RenderRequest(
                    message.getString("run_config"),
                    message.getString("assets_root"),
                )
            }
        }
    }

    companion object {
        /**
         * The tag the host logs under — the CLI's readiness wait filters
         * `logcat` on it.
         */
        const val TAG = "HydrolysisPreview"

        /**
         * The start outcomes the CLI waits for, each logged as
         * `<outcome>: stamp <stamp>` — keep them in step with
         * `HOST_SERVING`/`HOST_START_FAILED` on the CLI side.
         */
        const val SERVING = "preview host serving"
        const val START_FAILED = "preview host failed to start"

        /**
         * The abstract-domain name `adb forward` reaches this server
         * through — `localabstract:<name>` on the CLI side.
         */
        private const val SOCKET_NAME = "dev.waterui.hydrolysis.preview"

        /**
         * The wire schema this server speaks; the CLI refuses a greeting
         * naming another one.
         */
        private const val PROTOCOL_SCHEMA = 1

        /**
         * The request's bound after the greeting: a connected peer that
         * never sends one would otherwise pin this connection open, leaving
         * every later probe queued behind it.
         */
        private const val REQUEST_DEADLINE_MS = 30_000

        private const val RENDERED = "{\"type\":\"rendered\"}"

        private fun writeLine(output: OutputStream, line: String) {
            output.write("$line\n".toByteArray(Charsets.UTF_8))
            output.flush()
        }

        private fun readLine(input: InputStream): String {
            val line = ByteArrayOutputStream()
            while (true) {
                when (val byte = input.read()) {
                    -1 ->
                        throw EOFException(
                            "the preview channel closed before a request arrived",
                        )
                    '\n'.code -> return line.toString(Charsets.UTF_8.name())
                    else -> line.write(byte)
                }
            }
        }
    }
}
