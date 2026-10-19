package dev.waterui.hydrolysis

import android.os.Looper
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.Executor
import org.robolectric.Shadows.shadowOf

/**
 * A [HydrolysisEnvironment.syncExecutor] that holds the sync until the test
 * releases it: [runOffMain] runs it on a real worker thread, the way the
 * production executor does, then drains the main-looper hand-off.
 */
internal class CaptiveSyncWorker : Executor {
    private val queued = ArrayBlockingQueue<Runnable>(1)

    /** Syncs handed over and not yet run. */
    val pending: Int
        get() = queued.size

    override fun execute(command: Runnable) {
        queued.add(command)
    }

    fun runOffMain() {
        val worker = Thread(queued.remove())
        worker.start()
        worker.join()
        shadowOf(Looper.getMainLooper()).idle()
    }
}
