package dev.waterui.hydrolysis

import android.content.Context
import android.os.Build
import android.os.CpuHeadroomParams
import android.os.GpuHeadroomParams
import android.os.Handler
import android.os.PerformanceHintManager
import android.os.PowerManager
import android.os.Process
import android.os.WorkDuration
import android.os.health.SystemHealthManager
import androidx.annotation.RequiresApi
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit

/**
 * Android Dynamic Performance Framework (ADPF) coordinator for a
 * [HydrolysisSession].
 *
 * - Opens a [PerformanceHintManager.Session] lazily while continuous work,
 *   touch interaction, or an async Cherenkov GPU frame is active, binding the UI
 *   thread and (when provided) the `"cherenkov-render"` worker thread, and
 *   closes it immediately when idle, hidden, or destroyed.
 * - Reports each rendered frame's wall-clock work duration (reusing a single
 *   [WorkDuration] on API 35+, falling back to `reportActualWorkDuration(long)`
 *   on API 33–34), never thread CPU clocks.
 * - Observes [PowerManager] thermal status (event-driven listener on API 33+),
 *   thermal headroom (`addThermalHeadroomListener` on API 36+ with rate-limited
 *   background `getThermalHeadroom` fallback), and Android 16 CPU/GPU headroom
 *   ([SystemHealthManager] on API 36+) strictly off the frame-critical threads
 *   and only while active.
 */
internal class FramePerformance(
    context: Context,
    private val mainHandler: Handler,
    private val onHeadroomChanged: (Float, Float, Int, Float) -> Unit,
    private val hintClock: () -> Long = { System.nanoTime() },
) {
    private val uiTid: Int = Process.myTid()
    private val hintManager: PerformanceHintManager? =
        context.getSystemService(PerformanceHintManager::class.java)
    private val powerManager: PowerManager? =
        context.getSystemService(PowerManager::class.java)
    private val api36Probe: Api36HeadroomProbe? =
        if (Build.VERSION.SDK_INT >= 36) {
            context.getSystemService(SystemHealthManager::class.java)?.let { health ->
                Api36HeadroomProbe(health, powerManager) { cpu, gpu, thermalHeadroom ->
                    postHeadroomFromBackground(cpu, gpu, thermalHeadroom)
                }
            }
        } else {
            null
        }

    private val preferredUpdateRateNanos: Long =
        hintManager?.preferredUpdateRateNanos?.takeIf { it > 0L } ?: 0L
    private val workDuration: WorkDuration? =
        if (Build.VERSION.SDK_INT >= 35) WorkDuration() else null

    private var hintSession: PerformanceHintManager.Session? = null
    private var sessionThreads: IntArray = intArrayOf()
    private var activeTargetDurationNanos: Long = 0L
    private var lastTargetUpdateNanos: Long = 0L
    private var lastReportNanos: Long = 0L

    private var inFrameDispatch: Boolean = false
    private var deferredClose: Boolean = false
    private var pendingStartNanos: Long = -1L
    private var pendingUiCpuNanos: Long = 0L

    private var closed: Boolean = false
    private var activeForSampling: Boolean = false

    @Volatile
    private var latestThermalStatus: Int = PowerManager.THERMAL_STATUS_NONE
    @Volatile
    private var latestThermalHeadroom: Float = Float.NaN
    @Volatile
    private var latestCpuHeadroom: Float = Float.NaN
    @Volatile
    private var latestGpuHeadroom: Float = Float.NaN
    private var lastPublishedCpu: Float = Float.NaN
    private var lastPublishedGpu: Float = Float.NaN
    private var lastPublishedThermalStatus: Int = Int.MIN_VALUE
    private var lastPublishedThermalHeadroom: Float = Float.NaN

    private var pollExecutor: ScheduledExecutorService? = null
    private var pollFuture: ScheduledFuture<*>? = null
    private var thermalHeadroomSupported: Boolean = powerManager != null
    private var thermalStatusListenerRegistered: Boolean = false
    private var lastThermalPollMs: Long = 0L

    private val thermalStatusListener =
        PowerManager.OnThermalStatusChangedListener { status ->
            if (closed) return@OnThermalStatusChangedListener
            latestThermalStatus = status
            mainHandler.post { publishHeadroomIfChanged() }
        }

    init {
        powerManager?.let { pm ->
            latestThermalStatus = try {
                pm.currentThermalStatus
            } catch (_: RuntimeException) {
                PowerManager.THERMAL_STATUS_NONE
            }
            try {
                pm.addThermalStatusListener({ runnable -> mainHandler.post(runnable) }, thermalStatusListener)
                thermalStatusListenerRegistered = true
            } catch (_: RuntimeException) {
                thermalStatusListenerRegistered = false
            }
        }
    }

    /** Marks the start of a synchronous [FrameScheduler] frame transaction. */
    fun beginFrameDispatch() {
        inFrameDispatch = true
        deferredClose = false
    }

    /** Marks the end of a synchronous [FrameScheduler] frame transaction. */
    fun endFrameDispatch() {
        inFrameDispatch = false
        if (deferredClose && pendingStartNanos < 0L) {
            deferredClose = false
            closeHintSession()
        }
    }

    /**
     * Updates the active ADPF target budget (`<= 0` closes/suspends when any
     * outstanding frame finishes) and participating threads (`uiTid`, plus
     * `renderThreadTid` when `> 0`).
     */
    fun updateTarget(targetDurationNanos: Long, renderThreadTid: Int) {
        if (closed) return
        if (targetDurationNanos <= 0L) {
            setSamplingActive(false)
            if (inFrameDispatch || pendingStartNanos >= 0L) {
                deferredClose = true
            } else {
                closeHintSession()
            }
            return
        }

        deferredClose = false
        setSamplingActive(true)
        val threads = desiredThreads(renderThreadTid)
        if (Build.VERSION.SDK_INT >= 36) {
            api36Probe?.updateTids(threads)
        }

        val manager = hintManager ?: return
        val now = hintClock()
        var session = hintSession
        if (session == null) {
            session = manager.createHintSession(threads, targetDurationNanos) ?: return
            hintSession = session
            sessionThreads = threads
            activeTargetDurationNanos = targetDurationNanos
            lastTargetUpdateNanos = now
            return
        }

        if (!sessionThreads.contentEquals(threads)) {
            if (Build.VERSION.SDK_INT >= 34) {
                session.setThreads(threads)
                sessionThreads = threads
            } else {
                closeHintSession()
                hintSession = manager.createHintSession(threads, targetDurationNanos) ?: return
                sessionThreads = threads
                activeTargetDurationNanos = targetDurationNanos
                lastTargetUpdateNanos = now
                return
            }
        }

        if (activeTargetDurationNanos != targetDurationNanos &&
            (lastTargetUpdateNanos == 0L || now - lastTargetUpdateNanos >= preferredUpdateRateNanos)
        ) {
            session.updateTargetWorkDuration(targetDurationNanos)
            activeTargetDurationNanos = targetDurationNanos
            lastTargetUpdateNanos = now
        }
    }

    /** Records an asynchronous Cherenkov GPU frame in flight. */
    fun beginPendingFrame(startNanos: Long, uiCpuWallNanos: Long) {
        if (closed) return
        pendingStartNanos = startNanos
        pendingUiCpuNanos = uiCpuWallNanos.coerceAtLeast(1L)
    }

    /**
     * Finishes an asynchronous GPU frame and reports total wall duration plus
     * combined UI + `"cherenkov-render"` CPU wall duration.
     */
    fun completePendingFrame(renderCpuWallNanos: Long) {
        if (closed) return
        val startNanos = pendingStartNanos
        if (startNanos < 0L) return
        pendingStartNanos = -1L
        val totalWallNanos = (hintClock() - startNanos).coerceAtLeast(1L)
        val cpuWallNanos = (pendingUiCpuNanos + renderCpuWallNanos.coerceAtLeast(0L))
            .coerceIn(1L, totalWallNanos)
        reportDuration(startNanos, totalWallNanos, cpuWallNanos)
        if (deferredClose && !inFrameDispatch) {
            deferredClose = false
            closeHintSession()
        }
    }

    /** Reports a synchronous frame (HWUI or same-turn completion). */
    fun reportSynchronousFrame(startNanos: Long, wallNanos: Long) {
        if (closed) return
        val total = wallNanos.coerceAtLeast(1L)
        reportDuration(startNanos, total, total)
    }

    /** Releases the hint session, thermal listeners, and background poller. */
    fun close() {
        if (closed) return
        closed = true
        setSamplingActive(false)
        pendingStartNanos = -1L
        deferredClose = false
        closeHintSession()
        if (thermalStatusListenerRegistered) {
            thermalStatusListenerRegistered = false
            try {
                powerManager?.removeThermalStatusListener(thermalStatusListener)
            } catch (_: RuntimeException) {
                // Best-effort cleanup during teardown.
            }
        }
        if (Build.VERSION.SDK_INT >= 36) {
            api36Probe?.unregisterListener(powerManager)
        }
        pollExecutor?.shutdownNow()
        pollExecutor = null
    }

    private fun reportDuration(startNanos: Long, totalWallNanos: Long, cpuWallNanos: Long) {
        val session = hintSession ?: return
        val now = hintClock()
        if (preferredUpdateRateNanos > 0L &&
            lastReportNanos != 0L &&
            now - lastReportNanos < preferredUpdateRateNanos
        ) {
            return
        }
        lastReportNanos = now
        val duration = workDuration
        if (Build.VERSION.SDK_INT >= 35 && duration != null) {
            duration.workPeriodStartTimestampNanos = startNanos
            duration.actualTotalDurationNanos = totalWallNanos
            duration.actualCpuDurationNanos = cpuWallNanos
            session.reportActualWorkDuration(duration)
        } else {
            session.reportActualWorkDuration(totalWallNanos)
        }
    }

    private fun closeHintSession() {
        hintSession?.close()
        hintSession = null
        sessionThreads = intArrayOf()
        activeTargetDurationNanos = 0L
        lastTargetUpdateNanos = 0L
        lastReportNanos = 0L
    }

    private fun desiredThreads(renderThreadTid: Int): IntArray =
        if (renderThreadTid > 0 && renderThreadTid != uiTid) {
            intArrayOf(uiTid, renderThreadTid)
        } else {
            intArrayOf(uiTid)
        }

    private fun setSamplingActive(active: Boolean) {
        if (activeForSampling == active) return
        activeForSampling = active
        if (!active) {
            pollFuture?.cancel(false)
            pollFuture = null
            return
        }
        publishHeadroomIfChanged()
        val executor = ensurePollExecutor()
        if (Build.VERSION.SDK_INT >= 36) {
            api36Probe?.registerListenerIfSupported(powerManager, executor)
        }
        pollFuture = executor.schedule(
            { runBackgroundPollCycle() },
            0L,
            TimeUnit.MILLISECONDS,
        )
    }

    private fun ensurePollExecutor(): ScheduledExecutorService =
        pollExecutor ?: Executors.newSingleThreadScheduledExecutor { runnable ->
            Thread(runnable, "waterui-adpf-headroom").apply { isDaemon = true }
        }.also { pollExecutor = it }

    private fun runBackgroundPollCycle() {
        val nextDelayMs = sampleBackgroundHeadroom()
        if (closed || !activeForSampling || nextDelayMs == Long.MAX_VALUE) {
            return
        }
        val executor = pollExecutor ?: return
        pollFuture = executor.schedule(
            { runBackgroundPollCycle() },
            nextDelayMs.coerceAtLeast(MIN_POLL_INTERVAL_MS),
            TimeUnit.MILLISECONDS,
        )
    }

    private fun sampleBackgroundHeadroom(): Long {
        if (closed || !activeForSampling) return Long.MAX_VALUE
        val nowMs = System.currentTimeMillis()
        var cpu = latestCpuHeadroom
        var gpu = latestGpuHeadroom
        var thermal = latestThermalHeadroom
        var nextDelayMs = Long.MAX_VALUE

        if (Build.VERSION.SDK_INT >= 36) {
            api36Probe?.let { probe ->
                val sampled = probe.sample(nowMs)
                if (sampled != null) {
                    cpu = sampled.first
                    gpu = sampled.second
                }
                nextDelayMs = minOf(nextDelayMs, probe.nextDelayMs(nowMs))
            }
        }
        val needsThermalPoll = Build.VERSION.SDK_INT < 36 || api36Probe?.hasThermalListener != true
        if (needsThermalPoll && thermalHeadroomSupported) {
            val elapsed = nowMs - lastThermalPollMs
            if (lastThermalPollMs == 0L || elapsed >= THERMAL_POLL_INTERVAL_MS) {
                lastThermalPollMs = nowMs
                val pm = powerManager
                if (pm != null) {
                    try {
                        val reading = pm.getThermalHeadroom(THERMAL_FORECAST_SECONDS)
                        if (reading.isFinite() && reading >= 0f) {
                            thermal = reading
                        }
                    } catch (_: UnsupportedOperationException) {
                        thermalHeadroomSupported = false
                    } catch (_: SecurityException) {
                        thermalHeadroomSupported = false
                    }
                }
            }
            if (thermalHeadroomSupported) {
                val remaining = THERMAL_POLL_INTERVAL_MS - (nowMs - lastThermalPollMs)
                nextDelayMs = minOf(nextDelayMs, remaining.coerceAtLeast(MIN_POLL_INTERVAL_MS))
            }
        }
        postHeadroomFromBackground(cpu, gpu, thermal)
        return nextDelayMs
    }

    private fun postHeadroomFromBackground(cpu: Float, gpu: Float, thermalHeadroom: Float) {
        if (closed) return
        latestCpuHeadroom = cpu
        latestGpuHeadroom = gpu
        latestThermalHeadroom = thermalHeadroom
        mainHandler.post { publishHeadroomIfChanged() }
    }

    private fun publishHeadroomIfChanged() {
        if (closed) return
        val cpu = latestCpuHeadroom
        val gpu = latestGpuHeadroom
        val thermalStatus = latestThermalStatus
        val thermalHeadroom = latestThermalHeadroom
        if (cpu.toRawBits() == lastPublishedCpu.toRawBits() &&
            gpu.toRawBits() == lastPublishedGpu.toRawBits() &&
            thermalStatus == lastPublishedThermalStatus &&
            thermalHeadroom.toRawBits() == lastPublishedThermalHeadroom.toRawBits()
        ) {
            return
        }
        lastPublishedCpu = cpu
        lastPublishedGpu = gpu
        lastPublishedThermalStatus = thermalStatus
        lastPublishedThermalHeadroom = thermalHeadroom
        onHeadroomChanged(cpu, gpu, thermalStatus, thermalHeadroom)
    }

    private companion object {
        const val THERMAL_FORECAST_SECONDS = 10
        const val THERMAL_POLL_INTERVAL_MS = 10_000L
        const val MIN_POLL_INTERVAL_MS = 500L
    }
}

/**
 * Isolates API 36 (`SystemHealthManager` / `OnThermalHeadroomChangedListener`)
 * references behind `@RequiresApi(36)` so API 33–35 class verification never
 * resolves Android 16 types.
 */
@RequiresApi(36)
private class Api36HeadroomProbe(
    private val health: SystemHealthManager,
    powerManager: PowerManager?,
    private val onSample: (Float, Float, Float) -> Unit,
) {
    private var probedSupport = false
    private var cpuSupported = true
    private var gpuSupported = true
    private var cpuMinIntervalMs = 500L
    private var gpuMinIntervalMs = 500L
    private var lastCpuPollMs = 0L
    private var lastGpuPollMs = 0L
    @Volatile
    private var cpuParams: CpuHeadroomParams =
        CpuHeadroomParams.Builder()
            .setCalculationType(CpuHeadroomParams.CPU_HEADROOM_CALCULATION_TYPE_MIN)
            .build()
    private val gpuParams: GpuHeadroomParams =
        GpuHeadroomParams.Builder()
            .setCalculationType(GpuHeadroomParams.GPU_HEADROOM_CALCULATION_TYPE_MIN)
            .build()
    private var lastCpu = Float.NaN
    private var lastGpu = Float.NaN
    private var lastThermal = Float.NaN
    private var listenerRegistered = false
    private var thermalListenerSupported = powerManager != null

    val hasThermalListener: Boolean
        get() = listenerRegistered

    private val thermalHeadroomListener =
        PowerManager.OnThermalHeadroomChangedListener { headroom, forecastHeadroom, _, _ ->
            val effective = if (forecastHeadroom.isFinite()) forecastHeadroom else headroom
            if (effective.isFinite() && effective >= 0f) {
                lastThermal = effective
                onSample(lastCpu, lastGpu, lastThermal)
            }
        }

    fun registerListenerIfSupported(powerManager: PowerManager?, executor: ScheduledExecutorService) {
        if (listenerRegistered || !thermalListenerSupported || powerManager == null) return
        try {
            powerManager.addThermalHeadroomListener(executor, thermalHeadroomListener)
            listenerRegistered = true
        } catch (_: RuntimeException) {
            thermalListenerSupported = false
        }
    }

    fun unregisterListener(powerManager: PowerManager?) {
        if (!listenerRegistered || powerManager == null) return
        listenerRegistered = false
        try {
            powerManager.removeThermalHeadroomListener(thermalHeadroomListener)
        } catch (_: RuntimeException) {
            // Best-effort cleanup during teardown.
        }
    }

    fun updateTids(tids: IntArray) {
        val maxTids = try {
            health.maxCpuHeadroomTidsSize
        } catch (_: UnsupportedOperationException) {
            0
        } catch (_: SecurityException) {
            0
        }
        if (maxTids <= 0 || tids.isEmpty()) return
        val capped = if (tids.size > maxTids) tids.copyOf(maxTids) else tids
        cpuParams = CpuHeadroomParams.Builder(cpuParams).setTids(*capped).build()
    }

    fun nextDelayMs(nowMs: Long): Long {
        val cpuNext = if (cpuSupported) (cpuMinIntervalMs - (nowMs - lastCpuPollMs)).coerceAtLeast(500L) else Long.MAX_VALUE
        val gpuNext = if (gpuSupported) (gpuMinIntervalMs - (nowMs - lastGpuPollMs)).coerceAtLeast(500L) else Long.MAX_VALUE
        return minOf(cpuNext, gpuNext)
    }

    fun sample(nowMs: Long): Pair<Float, Float>? {
        ensureProbed()
        var changed = false
        if (cpuSupported && nowMs - lastCpuPollMs >= cpuMinIntervalMs) {
            lastCpuPollMs = nowMs
            try {
                val value = health.getCpuHeadroom(cpuParams)
                if (value.isFinite() && value >= 0f) {
                    lastCpu = value
                    changed = true
                }
            } catch (_: UnsupportedOperationException) {
                cpuSupported = false
            } catch (_: SecurityException) {
                cpuSupported = false
            }
        }
        if (gpuSupported && nowMs - lastGpuPollMs >= gpuMinIntervalMs) {
            lastGpuPollMs = nowMs
            try {
                val value = health.getGpuHeadroom(gpuParams)
                if (value.isFinite() && value >= 0f) {
                    lastGpu = value
                    changed = true
                }
            } catch (_: UnsupportedOperationException) {
                gpuSupported = false
            } catch (_: SecurityException) {
                gpuSupported = false
            }
        }
        return if (changed) lastCpu to lastGpu else null
    }

    private fun ensureProbed() {
        if (probedSupport) return
        probedSupport = true
        try {
            cpuMinIntervalMs = health.cpuHeadroomMinIntervalMillis.coerceAtLeast(500L)
        } catch (_: UnsupportedOperationException) {
            cpuSupported = false
        } catch (_: SecurityException) {
            cpuSupported = false
        }
        try {
            gpuMinIntervalMs = health.gpuHeadroomMinIntervalMillis.coerceAtLeast(500L)
        } catch (_: UnsupportedOperationException) {
            gpuSupported = false
        } catch (_: SecurityException) {
            gpuSupported = false
        }
    }
}
