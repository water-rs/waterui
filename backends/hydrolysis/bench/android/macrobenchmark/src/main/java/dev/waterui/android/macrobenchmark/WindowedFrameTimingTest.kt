package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.ExperimentalMetricApi
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

@OptIn(ExperimentalMetricApi::class)
class WindowedFrameTimingTest {
    private fun frame(
        start: Long,
        end: Long,
        expected: Long?,
        token: Long,
        present: Long?,
        type: String = "On-time Present",
        jank: String = "None",
    ) =
        WindowedFrame(
            start * 1_000_000,
            end * 1_000_000,
            expected?.times(1_000_000),
            token,
            present?.times(1_000_000),
            type,
            jank,
        )

    @Test
    fun windowExcludesSetupAndTrailingFramesAndDeduplicatesDisplayUpdates() {
        val result =
            windowedFrameMeasurements(
                    listOf(
                        frame(9, 12, 8, 10, 16),
                        frame(15, 20, 14, 11, 25),
                        frame(16, 22, 14, 11, 25),
                        frame(30, 33, 29, 0, null, "Dropped Frame"),
                        frame(30, 34, 29, 12, 40, "Late Present", "App Deadline Missed"),
                        frame(45, 48, 44, 13, 55),
                    ),
                    10_000_000,
                    50_000_000,
                )
                .associate { it.name to it.data }
        assertEquals(listOf(8.0, 5.0), result["windowFrameDurationMs"])
        assertEquals(listOf(11.0, 11.0), result["windowFramePresentationLatencyMs"])
        assertEquals(listOf(15.0), result["windowFramePresentIntervalMs"])
        assertEquals(listOf(2.0), result["windowPresentedFrameCount"])
        assertEquals(listOf(1.0), result["windowDroppedSurfaceFrameCount"])
        assertEquals(listOf(1.0), result["windowLateFrameCount"])
        assertEquals(listOf(1.0), result["windowJankyFrameCount"])
        assertEquals(listOf(1.0), result["windowBoundarySurfaceFrameCount"])
    }

    @Test
    fun missingPresentationFailsInsteadOfUsingAppCompletion() {
        assertThrows(IllegalStateException::class.java) {
            windowedFrameMeasurements(
                listOf(frame(15, 20, 14, 11, null)),
                10_000_000,
                50_000_000,
            )
        }
    }
}
