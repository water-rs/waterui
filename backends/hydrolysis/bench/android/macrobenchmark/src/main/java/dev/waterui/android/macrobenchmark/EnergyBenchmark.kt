package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.ExperimentalMetricApi
import androidx.benchmark.macro.PowerMetric
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import dev.waterui.android.bench.Journey
import dev.waterui.android.bench.SuiteRegistry
import dev.waterui.android.bench.loadInteractionSpec
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * System-wide energy per section 6: ODPM rails integrated over the same
 * scripted journey, ending with an idle segment so joules/script and idle
 * watts are read from the same round (energy/frame is undefined at idle —
 * frame counts land in the frame record, not here).
 *
 * PowerMetric requires a physical device with ODPM rails (Pixel 6+); the
 * harness's ODPM validation step confirms rails are nonzero before trusting
 * these numbers.
 *
 * Instrumentation args: fixture, script_seconds (45), idle_seconds (15),
 * iterations (1).
 */
@OptIn(ExperimentalMetricApi::class)
@RunWith(androidx.test.ext.junit.runners.AndroidJUnit4::class)
class EnergyBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    private val device: UiDevice =
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun energy() {
        val args = InstrumentationRegistry.getArguments()
        val fixture = args.getString("fixture") ?: "suite"
        val scriptSeconds = args.getString("script_seconds")?.toIntOrNull() ?: 45
        val idleSeconds = args.getString("idle_seconds")?.toIntOrNull() ?: 15
        val iterations = args.getString("iterations")?.toIntOrNull() ?: 1
        val spec = loadInteractionSpec(
            InstrumentationRegistry.getInstrumentation().context,
        )
        benchmarkRule.measureRepeated(
            packageName = SuiteRegistry.packageForFixture(fixture),
            metrics = listOf(PowerMetric(PowerMetric.Energy())),
            compilationMode = CompilationMode.Partial(
                BaselineProfileMode.Require,
            ),
            iterations = iterations,
            startupMode = StartupMode.WARM,
            setupBlock = { pressHome() },
        ) {
            startActivityAndWait { intent ->
                intent.putExtra("E2EExample", SuiteRegistry.exampleForFixture(fixture))
                intent.putExtra("waterui.env.WATERUI_DISABLE_DYNAMIC_COLORS", "1")
            }
            device.wait(Until.hasObject(By.res("screen-root")), 10_000)
            Journey.runFor(device, spec, fixture, scriptSeconds)
            // Idle segment: the app sits presented but untouched so the round
            // also reports the fixture's idle draw.
            Thread.sleep(idleSeconds * 1000L)
        }
    }
}
