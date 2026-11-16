package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.ExperimentalMetricApi
import androidx.benchmark.macro.PowerCategory
import androidx.benchmark.macro.PowerCategoryDisplayLevel
import androidx.benchmark.macro.PowerMetric
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import dev.waterui.android.bench.Journey
import dev.waterui.android.bench.loadInteractionSpec
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * System-wide ODPM energy over a fixed interaction window, with idle power measured separately
 * after the same journey has completed and settled. An in-flight gesture never enters the idle
 * window. Energy/frame is undefined at idle.
 *
 * PowerMetric requires a physical device with ODPM rails (Pixel 6+); the harness's ODPM validation
 * step confirms rails are nonzero before trusting these numbers.
 *
 * Instrumentation args: fixture, script_seconds (45), idle_seconds (15), iterations (1).
 */
@OptIn(ExperimentalMetricApi::class)
@RunWith(androidx.test.ext.junit.runners.AndroidJUnit4::class)
class EnergyBenchmark {

    @get:Rule val benchmarkRule = MacrobenchmarkRule()

    private val device: UiDevice =
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun energy() =
        measurePower(PowerMetric(PowerMetric.Energy(railCategories()))) { replay, scriptSeconds, _
            ->
            replay.measure(scriptSeconds)
        }

    @Test
    fun idle() =
        measurePower(
            PowerMetric(PowerMetric.Power(railCategories())),
            prepare = { replay, scriptSeconds ->
                replay.measure(scriptSeconds)
                replay.finish()
                device.waitForIdle(5000)
            },
        ) { _, _, idleSeconds ->
            Journey.idleFor(idleSeconds)
        }

    private fun railCategories() =
        PowerCategory.entries.associateWith { PowerCategoryDisplayLevel.BREAKDOWN }

    private fun measurePower(
        metric: PowerMetric,
        prepare: (Journey.TimedRun, Int) -> Unit = { _, _ -> },
        measure: (Journey.TimedRun, Int, Int) -> Unit,
    ) {
        val args = InstrumentationRegistry.getArguments()
        val fixture = args.getString("fixture") ?: "suite"
        val scriptSeconds = args.getString("script_seconds")?.toInt() ?: 45
        val idleSeconds = args.getString("idle_seconds")?.toInt() ?: 15
        val iterations = args.getString("iterations")?.toInt() ?: 1
        // Configurable bound for a final synchronous UIAutomator action to finish.
        val completionTimeout = args.getString("replay_finish_timeout_seconds")?.toLong() ?: 30L
        val spec = loadInteractionSpec(InstrumentationRegistry.getInstrumentation().context)
        BenchmarkTarget(args, fixture).use { target ->
            Journey.TimedRun(device, spec, fixture, completionTimeout).use { replay ->
                benchmarkRule.measureRepeated(
                    packageName = target.packageName,
                    metrics = listOf(metric),
                    compilationMode = target.compilationMode,
                    iterations = iterations,
                    startupMode = StartupMode.WARM,
                    setupBlock = {
                        replay.finish()
                        target.prepareLaunch()
                        Journey.prepareDevice(device)
                        pressHome()
                        startActivityAndWait { intent -> target.configureLaunch(intent) }
                        target.awaitContent(device)
                        prepare(replay, scriptSeconds)
                    },
                ) {
                    measure(replay, scriptSeconds, idleSeconds)
                }
            }
        }
    }
}
