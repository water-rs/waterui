package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.ExperimentalMetricApi
import androidx.benchmark.macro.Metric.Measurement
import androidx.benchmark.macro.TraceMetric
import androidx.benchmark.traceprocessor.ExperimentalTraceProcessorApi
import androidx.benchmark.traceprocessor.TraceProcessor
import androidx.test.platform.app.InstrumentationRegistry

/** FrameTimeline's app end is GPU/post completion; SurfaceFlinger's end is presentation. */
@OptIn(ExperimentalMetricApi::class, ExperimentalTraceProcessorApi::class)
class WindowedFrameTimingMetric : TraceMetric() {
    override fun getMeasurements(
        captureInfo: CaptureInfo,
        traceSession: TraceProcessor.Session,
    ): List<Measurement> {
        val window =
            traceSession
                .querySlices("measureBlock", packageName = captureInfo.testPackageName)
                .single()
        val sql =
            InstrumentationRegistry.getInstrumentation()
                .context
                .assets
                .open("windowed_frames.sql")
                .bufferedReader()
                .use { it.readText() }
        val frames =
            traceSession
                .query(sql)
                .filter { it.string("process_name") == captureInfo.targetPackageName }
                .map {
                    WindowedFrame(
                        it.long("actual_start_ns"),
                        it.long("actual_end_ns"),
                        it.nullableLong("expected_start_ns"),
                        it.long("display_frame_token"),
                        it.nullableLong("present_ns"),
                        it.string("present_type"),
                        it.string("jank_type"),
                    )
                }
                .toList()
        return windowedFrameMeasurements(frames, window.ts, window.endTs)
    }
}

internal data class WindowedFrame(
    val actualStartNs: Long,
    val actualEndNs: Long,
    val expectedStartNs: Long?,
    val displayToken: Long,
    val presentNs: Long?,
    val presentType: String,
    val jankType: String,
)

@OptIn(ExperimentalMetricApi::class)
internal fun windowedFrameMeasurements(
    frames: List<WindowedFrame>,
    startNs: Long,
    endNs: Long,
): List<Measurement> {
    val started = frames.filter { it.actualStartNs >= startNs && it.actualStartNs < endNs }
    val dropped = started.filter { it.presentType == "Dropped Frame" }
    val renderable = started.filter { it.presentType != "Dropped Frame" }
    renderable.forEach {
        checkNotNull(it.presentNs) {
            "Missing SurfaceFlinger presentation for frame ${it.displayToken}"
        }
        checkNotNull(it.expectedStartNs) {
            "Missing expected timeline for frame ${it.displayToken}"
        }
    }
    // Frames beginning before the window and those presented after it cannot enter its samples.
    val presented =
        renderable
            .filter { it.expectedStartNs!! >= startNs && it.presentNs!! < endNs }
            .groupBy { it.displayToken }
            .values
            .sortedBy { it.first().presentNs }
    check(presented.isNotEmpty()) { "No target frames presented inside measureBlock" }
    val appDurations = presented.map { layers ->
        layers.maxOf { it.actualEndNs - it.expectedStartNs!! } / 1_000_000.0
    }
    val presentLatencies = presented.map { layers ->
        layers.maxOf { it.presentNs!! - it.expectedStartNs!! } / 1_000_000.0
    }
    val intervals = presented.zipWithNext { a, b ->
        (b.first().presentNs!! - a.first().presentNs!!) / 1_000_000.0
    }
    return buildList {
        add(Measurement("windowFrameDurationMs", appDurations))
        add(Measurement("windowFramePresentationLatencyMs", presentLatencies))
        if (intervals.isNotEmpty()) add(Measurement("windowFramePresentIntervalMs", intervals))
        add(Measurement("windowPresentedFrameCount", presented.size.toDouble()))
        add(Measurement("windowDroppedSurfaceFrameCount", dropped.size.toDouble()))
        add(
            Measurement(
                "windowLateFrameCount",
                presented
                    .count { layers ->
                        layers.any { it.presentType == "Late Present" }
                    }
                    .toDouble(),
            )
        )
        add(
            Measurement(
                "windowJankyFrameCount",
                presented
                    .count { layers ->
                        layers.any { it.jankType != "None" }
                    }
                    .toDouble(),
            )
        )
        add(
            Measurement(
                "windowBoundarySurfaceFrameCount",
                (renderable.size - presented.sumOf { it.size }).toDouble(),
            )
        )
        add(Measurement("windowMaxFrameDurationMs", appDurations.max()))
        add(Measurement("windowMaxPresentationLatencyMs", presentLatencies.max()))
    }
}
