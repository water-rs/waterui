package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.ExperimentalMetricApi
import androidx.benchmark.macro.FrameTimingGfxInfoMetric
import androidx.benchmark.macro.FrameTimingMetric
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.TraceSectionMetric
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import dev.waterui.android.bench.Journey
import dev.waterui.android.bench.loadInteractionSpec
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The fixed 60-second scripted frame round from section 6.
 *
 * WindowedFrameTimingMetric joins target frames to SurfaceFlinger and limits samples to
 * measureBlock. Raw AndroidX CPU and gfxinfo summaries remain available for cross-checking; they do
 * not establish the window's surface-presentation coverage on their own.
 *
 * Instrumentation args: fixture (default "suite"), script_seconds (60), iterations (1).
 */
@OptIn(ExperimentalMetricApi::class)
@RunWith(androidx.test.ext.junit.runners.AndroidJUnit4::class)
class FrameBenchmark {

    @get:Rule val benchmarkRule = MacrobenchmarkRule()

    private val device: UiDevice =
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun frames() {
        val args = InstrumentationRegistry.getArguments()
        val fixture = args.getString("fixture") ?: "suite"
        BenchmarkTarget(args, fixture).use { target ->
            val scriptSeconds = args.getString("script_seconds")?.toInt() ?: 60
            val iterations = args.getString("iterations")?.toInt() ?: 1
            val spec = loadInteractionSpec(InstrumentationRegistry.getInstrumentation().context)
            // Configurable bound for a final synchronous UIAutomator action to finish.
            val completionTimeout = args.getString("replay_finish_timeout_seconds")?.toLong() ?: 30L
            Journey.TimedRun(device, spec, fixture, completionTimeout).use { replay ->
                benchmarkRule.measureRepeated(
                    packageName = target.packageName,
                    metrics =
                        listOf(
                            WindowedFrameTimingMetric(),
                            FrameTimingMetric(),
                            FrameTimingGfxInfoMetric(),
                            TraceSectionMetric("measureBlock", targetPackageOnly = false),
                        ),
                    compilationMode = target.compilationMode,
                    iterations = iterations,
                    startupMode = StartupMode.WARM,
                    setupBlock = {
                        replay.finish()
                        target.prepareLaunch()
                        Journey.prepareDevice(device)
                        pressHome()
                        startActivityAndWait { intent ->
                            target.configureLaunch(intent, markers = true)
                        }
                        target.awaitContent(device)
                    },
                ) {
                    replay.measure(scriptSeconds)
                }
            }
        }
    }
}
