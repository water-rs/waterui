package dev.waterui.hydrolysis

import android.content.Context
import android.os.Build
import android.os.PerformanceHintManager
import android.os.Process
import android.util.Log

/** One host's frame workload, including the Cherenkov worker it actually uses. */
internal class FramePerformance(context: Context) : AutoCloseable {
    private val manager = context.getSystemService(PerformanceHintManager::class.java)
    private var supported = manager != null
    private var session: PerformanceHintManager.Session? = null
    private var worker = 0
    private var targetNanos = 0L
    private var reportedTargetNanos = 0L

    init {
        if (!supported) Log.i("hydrolysis", "Performance hint manager is unavailable on this device")
    }

    fun setRefreshRate(hz: Float) {
        targetNanos = if (hz > 0f) (1_000_000_000.0 / hz).toLong() else 0L
        if (targetNanos == 0L) close()
    }

    /** Elapsed time for the whole thread-group task, not CPU execution time alone. */
    fun report(renderThread: Int, elapsedNanos: Long) {
        if (!supported || targetNanos == 0L) return
        if (session == null || worker != renderThread) {
            close()
            val threads = if (renderThread == 0) intArrayOf(Process.myTid())
                else intArrayOf(Process.myTid(), renderThread)
            val created = manager!!.createHintSession(threads, targetNanos)
            if (created == null) {
                supported = false
                Log.i("hydrolysis", "Performance hint sessions are unavailable on this device")
                return
            }
            if (Build.VERSION.SDK_INT >= 35) created.setPreferPowerEfficiency(true)
            session = created
            worker = renderThread
            reportedTargetNanos = targetNanos
        }
        val active = checkNotNull(session)
        if (reportedTargetNanos != targetNanos) {
            active.updateTargetWorkDuration(targetNanos)
            reportedTargetNanos = targetNanos
        }
        active.reportActualWorkDuration(elapsedNanos)
    }

    override fun close() {
        session?.close()
        session = null
    }
}
