package dev.waterui.hydrolysis

import android.content.Context
import android.content.pm.ApplicationInfo
import android.system.Os
import java.util.concurrent.Executors

/**
 * Loads a launch-environment handoff off the main thread and hands the
 * embedding an apply closure on it. Instance-owned: the generated
 * `MainActivity` constructs one per launch, and tests inject their own
 * [environmentApplier] instead of touching the JVM's real environment.
 * Each preparation owns a short-lived executor, claims the handoff on the
 * worker, shuts the executor down, and delivers the closure through the
 * application context's main executor.
 */
class LaunchEnvironmentLoader internal constructor(
    context: Context,
    private val environmentApplier: (String, String) -> Unit,
    private val claimBarrier: () -> Unit = {},
) {

    private val appContext = context.applicationContext

    constructor(context: Context) : this(context, nativeApplier)

    /**
     * Read and claim the handoff `id` on a worker, then deliver the apply
     * closure through the app context's main executor. A missing id is a
     * launch with no environment — the closure is empty. A read or claim
     * failure becomes an apply-time throw carrying the store's fixed
     * message.
     */
    fun prepare(id: String?, callback: (() -> Unit) -> Unit) {
        if (id == null) {
            callback {}
            return
        }
        val executor = Executors.newSingleThreadExecutor()
        executor.execute {
            val apply: () -> Unit = try {
                val values = LaunchEnvironmentStore.claim(appContext, id, claimBarrier)
                val debuggable =
                    appContext.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
                val bound: () -> Unit = { values.applyTo(debuggable, environmentApplier) }
                bound
            } catch (error: Exception) {
                val failure = error
                ({ throw failure })
            }
            appContext.mainExecutor.execute { callback(apply) }
        }
        // The queued task runs to completion; shutting down now frees the
        // pool as soon as it finishes — a failed enqueue or worker error
        // can't strand it.
        executor.shutdown()
    }

    private companion object {

        /** The production apply: `Os.setenv` with a key-only failure. */
        val nativeApplier: (String, String) -> Unit = { key, value ->
            try {
                Os.setenv(key, value, true)
            } catch (error: Exception) {
                throw IllegalStateException(
                    "could not apply launch environment variable $key",
                )
            }
        }
    }
}
