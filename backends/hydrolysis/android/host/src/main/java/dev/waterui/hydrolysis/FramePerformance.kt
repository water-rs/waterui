package dev.waterui.hydrolysis

import android.content.Context
import android.os.Build
import android.os.PerformanceHintManager
import android.os.Process
import android.os.WorkDuration
import android.util.Log

/** One GPU host's CPU work, with waits kept in the separate elapsed interval. */
internal class FramePerformance(context: Context) : AutoCloseable {
    private val manager = context.getSystemService(PerformanceHintManager::class.java)
    private var supported = manager != null
    private val work = if (Build.VERSION.SDK_INT >= 35) WorkDuration() else null
    private var session: PerformanceHintManager.Session? = null
    private var worker = 0
    private var targetNanos = 0L
    private var reportedTargetNanos = 0L

    fun setRefreshRate(hz: Float) {
        targetNanos = if (hz > 0f) (1_000_000_000.0 / hz).toLong() else 0L
        if (targetNanos == 0L) close()
    }

    fun report(renderThread: Int, started: Long, elapsed: Long, cpu: Long) {
        if (Build.VERSION.SDK_INT < 35 || !supported || targetNanos == 0L || renderThread == 0) return
        if (session == null || worker != renderThread) {
            close()
            val created = checkNotNull(manager).createHintSession(
                intArrayOf(Process.myTid(), renderThread), targetNanos,
            )
            if (created == null) {
                supported = false
                Log.i("hydrolysis", "Performance hint sessions are unavailable on this device")
                return
            }
            session = created
            worker = renderThread
            reportedTargetNanos = targetNanos
            // A new worker has no CPU-clock sample from the start of this frame.
            return
        }
        val active = checkNotNull(session)
        if (reportedTargetNanos != targetNanos) {
            active.updateTargetWorkDuration(targetNanos)
            reportedTargetNanos = targetNanos
        }
        val duration = checkNotNull(work)
        duration.setWorkPeriodStartTimestampNanos(started)
        duration.setActualTotalDurationNanos(elapsed)
        duration.setActualCpuDurationNanos(cpu)
        // GPU time is not sampled by the runtime; this does not claim zero GPU work.
        duration.setActualGpuDurationNanos(0)
        active.reportActualWorkDuration(duration)
    }

    override fun close() {
        session?.close()
        session = null
    }
}
