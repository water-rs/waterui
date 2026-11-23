package dev.waterui.hydrolysis

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PerformanceHintManager
import android.os.WorkDuration
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.annotation.internal.DoNotInstrument
import org.robolectric.shadow.api.Shadow

@RunWith(RobolectricTestRunner::class)
@Config(
    sdk = [35],
    shadows = [
        ShadowNativeBridge::class,
        ShadowPerformanceHintManager::class,
        ShadowPerformanceHintSession::class,
    ],
    instrumentedPackages = ["dev.waterui.hydrolysis"],
)
@DoNotInstrument
class FramePerformanceTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    @Before
    @Suppress("DEPRECATION")
    fun resetShadows() {
        ShadowNativeBridge.reset()
        ShadowPerformanceHintManager.reset()
        ShadowPerformanceHintSession.reset()
        shadowOf(RuntimeEnvironment.getApplication()).setSystemService(
            Context.PERFORMANCE_HINT_SERVICE,
            Shadow.newInstanceOf(PerformanceHintManager::class.java),
        )
    }

    @Test
    fun hintSessionOpensForActiveTargetUpdatesThreadsAndClosesOnIdle() {
        var nowNanos = 1_000_000_000L
        val performance = FramePerformance(
            context = context,
            mainHandler = Handler(Looper.getMainLooper()),
            onHeadroomChanged = { _, _, _, _ -> },
            hintClock = { nowNanos },
        )

        performance.updateTarget(targetDurationNanos = 8_333_333L, renderThreadTid = 0)
        assertEquals(1, ShadowPerformanceHintManager.createdTargets.size)
        assertEquals(8_333_333L, ShadowPerformanceHintManager.createdTargets.single())
        assertEquals(0, ShadowPerformanceHintSession.closedCount)

        nowNanos += 10_000_000L
        performance.updateTarget(targetDurationNanos = 8_333_333L, renderThreadTid = 4242)
        assertEquals(1, ShadowPerformanceHintSession.threadUpdates.size)
        assertTrue(ShadowPerformanceHintSession.threadUpdates.single().contains(4242))

        nowNanos += 20_000_000L
        performance.updateTarget(targetDurationNanos = 16_666_666L, renderThreadTid = 4242)
        assertEquals(listOf(16_666_666L), ShadowPerformanceHintSession.targetUpdates)

        performance.reportSynchronousFrame(startNanos = nowNanos - 2_000_000L, wallNanos = 2_000_000L)
        assertEquals(1, ShadowPerformanceHintSession.reportedTotalWallNanos.size)
        assertEquals(2_000_000L, ShadowPerformanceHintSession.reportedTotalWallNanos.single())
        assertEquals(2_000_000L, ShadowPerformanceHintSession.reportedCpuWallNanos.single())
        assertEquals(0L, ShadowPerformanceHintSession.reportedGpuWallNanos.single())

        performance.updateTarget(targetDurationNanos = 0L, renderThreadTid = 0)
        assertEquals(1, ShadowPerformanceHintSession.closedCount)
        performance.close()
    }

    @Test
    fun syncAndAsyncFramesReportWallWorkDurationsThroughSessionAndRespectIdle() {
        val painter = RecordingPainter()
        val session = HydrolysisSession(context, painter = painter, onCloseRequested = {})
        session.onNativePerformanceTarget(
            targetDurationNanos = 8_333_333L,
            renderThreadTid = 999,
            maxRefreshHz = 90f,
        )
        assertEquals(listOf(90f), painter.maxRefreshRates)
        assertEquals(1, ShadowPerformanceHintManager.createdTargets.size)

        // Frame without RENDERED_WORK skips hint reporting.
        ShadowNativeBridge.onFrame = { 0L }
        session.pumpOnceForTest()
        assertEquals(0, ShadowPerformanceHintSession.reportedTotalWallNanos.size)

        // Synchronous frame with RENDERED_WORK reports wall time immediately.
        ShadowNativeBridge.onFrame = { 8L }
        session.pumpOnceForTest()
        assertEquals(1, ShadowPerformanceHintSession.reportedTotalWallNanos.size)

        // Async GPU frame defers reporting until onNativeFrameCompleted().
        ShadowPerformanceHintSession.reportedTotalWallNanos.clear()
        ShadowNativeBridge.onFrame = { 8L or 16L }
        ShadowNativeBridge.completeFrameCpuNanos = 500_000L
        session.pumpOnceForTest()
        assertEquals(0, ShadowPerformanceHintSession.reportedTotalWallNanos.size)

        session.onNativeFrameCompleted()
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(1, ShadowPerformanceHintSession.reportedTotalWallNanos.size)
        assertTrue(ShadowPerformanceHintSession.reportedTotalWallNanos.single() >= 1L)

        // Transitioning idle closes the hint session cleanly.
        session.onNativePerformanceTarget(targetDurationNanos = 0L, renderThreadTid = 0, maxRefreshHz = 0f)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(1, ShadowPerformanceHintSession.closedCount)
        session.destroy()
        shadowOf(Looper.getMainLooper()).idle()
    }

    @Test
    @Config(sdk = [33])
    fun api33FallsBackToScalarWorkDurationAndRecreatesSessionOnThreadChange() {
        var nowNanos = 2_000_000_000L
        val performance = FramePerformance(
            context = context,
            mainHandler = Handler(Looper.getMainLooper()),
            onHeadroomChanged = { _, _, _, _ -> },
            hintClock = { nowNanos },
        )

        performance.updateTarget(targetDurationNanos = 8_333_333L, renderThreadTid = 0)
        assertEquals(1, ShadowPerformanceHintManager.createdTargets.size)

        performance.reportSynchronousFrame(startNanos = nowNanos - 1_500_000L, wallNanos = 1_500_000L)
        assertEquals(listOf(1_500_000L), ShadowPerformanceHintSession.reportedScalarDurations)

        // API 33 has no setThreads; changing renderThreadTid recreates the hint session.
        performance.updateTarget(targetDurationNanos = 8_333_333L, renderThreadTid = 777)
        assertEquals(1, ShadowPerformanceHintSession.closedCount)
        assertEquals(2, ShadowPerformanceHintManager.createdTargets.size)
        assertArrayEquals(intArrayOf(android.os.Process.myTid(), 777), ShadowPerformanceHintManager.lastCreatedThreads)
        performance.close()
    }

    @Test
    fun respectsPreferredUpdateRateNanosForReportsAndTargetChanges() {
        ShadowPerformanceHintManager.preferredUpdateRateNanos = 10_000_000L
        var nowNanos = 5_000_000_000L
        val performance = FramePerformance(
            context = context,
            mainHandler = Handler(Looper.getMainLooper()),
            onHeadroomChanged = { _, _, _, _ -> },
            hintClock = { nowNanos },
        )

        performance.updateTarget(targetDurationNanos = 8_333_333L, renderThreadTid = 0)
        performance.reportSynchronousFrame(startNanos = nowNanos - 1_000_000L, wallNanos = 1_000_000L)
        assertEquals(1, ShadowPerformanceHintSession.reportedTotalWallNanos.size)

        // Within preferredUpdateRateNanos, duplicate reports and target updates are throttled.
        nowNanos += 4_000_000L
        performance.updateTarget(targetDurationNanos = 16_666_666L, renderThreadTid = 0)
        performance.reportSynchronousFrame(startNanos = nowNanos - 1_000_000L, wallNanos = 1_000_000L)
        assertEquals(1, ShadowPerformanceHintSession.reportedTotalWallNanos.size)
        assertEquals(0, ShadowPerformanceHintSession.targetUpdates.size)

        // Once preferredUpdateRateNanos elapses, both report and target update proceed.
        nowNanos += 10_000_000L
        performance.updateTarget(targetDurationNanos = 16_666_666L, renderThreadTid = 0)
        performance.reportSynchronousFrame(startNanos = nowNanos - 2_000_000L, wallNanos = 2_000_000L)
        assertEquals(2, ShadowPerformanceHintSession.reportedTotalWallNanos.size)
        assertEquals(listOf(16_666_666L), ShadowPerformanceHintSession.targetUpdates)
        performance.close()
    }

    @Test
    fun idleTargetDuringCancelledPendingAsyncFrameClosesSessionImmediately() {
        var nowNanos = 3_000_000_000L
        val performance = FramePerformance(
            context = context,
            mainHandler = Handler(Looper.getMainLooper()),
            onHeadroomChanged = { _, _, _, _ -> },
            hintClock = { nowNanos },
        )

        performance.updateTarget(targetDurationNanos = 8_333_333L, renderThreadTid = 42)
        performance.beginPendingFrame(startNanos = nowNanos, uiCpuWallNanos = 500_000L)

        // Detach/background cancels the GPU frame before completion: outside dispatch/completion,
        // updateTarget(0) must close immediately instead of leaking open.
        performance.updateTarget(targetDurationNanos = 0L, renderThreadTid = 0)
        assertEquals(1, ShadowPerformanceHintSession.closedCount)

        nowNanos += 2_000_000L
        performance.completePendingFrame(renderCpuWallNanos = 300_000L)
        assertEquals(0, ShadowPerformanceHintSession.reportedTotalWallNanos.size)
        assertEquals(1, ShadowPerformanceHintSession.closedCount)

        // Inside completion, updateTarget(0) defers close until the completed frame reports.
        performance.updateTarget(targetDurationNanos = 8_333_333L, renderThreadTid = 42)
        performance.beginPendingFrame(startNanos = nowNanos, uiCpuWallNanos = 400_000L)
        nowNanos += 2_000_000L
        performance.beginFrameCompletion()
        performance.updateTarget(targetDurationNanos = 0L, renderThreadTid = 0)
        assertEquals(1, ShadowPerformanceHintSession.closedCount)
        performance.completePendingFrame(renderCpuWallNanos = 300_000L)
        performance.endFrameCompletion()
        assertEquals(1, ShadowPerformanceHintSession.reportedTotalWallNanos.size)
        assertEquals(2, ShadowPerformanceHintSession.closedCount)

        performance.close()
    }

    private fun HydrolysisSession.pumpOnceForTest() {
        val schedulerField = HydrolysisSession::class.java.declaredFields.single { field ->
            field.type == FrameScheduler::class.java
        }.apply { isAccessible = true }
        val scheduler = schedulerField.get(this)
        val pump = FrameScheduler::class.java.declaredMethods.single { method ->
            method.name == "pump" && !method.isSynthetic && method.parameterCount == 3
        }.apply { isAccessible = true }
        pump.invoke(scheduler, "test", null, null)
        shadowOf(Looper.getMainLooper()).idle()
    }

    private class RecordingPainter : SessionPainter {
        val maxRefreshRates = mutableListOf<Float>()

        override fun attach(host: HydrolysisHostView) {}

        override fun detach(host: HydrolysisHostView) {}

        override fun setHighRefresh(active: Boolean) {}

        override fun setMaxRefreshRate(maxHz: Float) {
            maxRefreshRates += maxHz
        }

        override fun destroy() {}
    }
}

@Implements(PerformanceHintManager::class)
class ShadowPerformanceHintManager {
    @Implementation
    fun getPreferredUpdateRateNanos(): Long = preferredUpdateRateNanos

    @Implementation
    fun createHintSession(tids: IntArray, initialTargetWorkDurationNanos: Long): PerformanceHintManager.Session {
        lastCreatedThreads = tids.copyOf()
        createdTargets += initialTargetWorkDurationNanos
        return Shadow.newInstanceOf(PerformanceHintManager.Session::class.java)
    }

    companion object {
        var preferredUpdateRateNanos = 0L
        val createdTargets = mutableListOf<Long>()
        var lastCreatedThreads: IntArray = intArrayOf()

        fun reset() {
            preferredUpdateRateNanos = 0L
            createdTargets.clear()
            lastCreatedThreads = intArrayOf()
        }
    }
}

@Implements(PerformanceHintManager.Session::class)
class ShadowPerformanceHintSession {
    @Implementation
    fun updateTargetWorkDuration(targetDurationNanos: Long) {
        targetUpdates += targetDurationNanos
    }

    @Implementation
    fun setThreads(tids: IntArray) {
        threadUpdates += tids.copyOf()
    }

    @Implementation
    fun reportActualWorkDuration(actualDurationNanos: Long) {
        reportedScalarDurations += actualDurationNanos
    }

    @Implementation
    fun reportActualWorkDuration(workDuration: WorkDuration) {
        reportedTotalWallNanos += workDuration.actualTotalDurationNanos
        reportedCpuWallNanos += workDuration.actualCpuDurationNanos
        reportedGpuWallNanos += workDuration.actualGpuDurationNanos
    }

    @Implementation
    fun close() {
        closedCount += 1
    }

    companion object {
        val targetUpdates = mutableListOf<Long>()
        val threadUpdates = mutableListOf<IntArray>()
        val reportedScalarDurations = mutableListOf<Long>()
        val reportedTotalWallNanos = mutableListOf<Long>()
        val reportedCpuWallNanos = mutableListOf<Long>()
        val reportedGpuWallNanos = mutableListOf<Long>()
        var closedCount = 0

        fun reset() {
            targetUpdates.clear()
            threadUpdates.clear()
            reportedScalarDurations.clear()
            reportedTotalWallNanos.clear()
            reportedCpuWallNanos.clear()
            reportedGpuWallNanos.clear()
            closedCount = 0
        }
    }
}
