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
 * FrameTimingMetric reports per-frame CPU presentation data and FrameTimingGfxInfoMetric the
 * gfxinfo percentiles; the shared journey loops until the script window closes so scroll/animation
 * demand runs the whole round at the display's demanded cadence. run.py pairs these records with
 * the Perfetto frametimeline trace for surface-level coverage — Compose draws through the decor
 * window, but the gate asserts coverage exists regardless of painter.
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
