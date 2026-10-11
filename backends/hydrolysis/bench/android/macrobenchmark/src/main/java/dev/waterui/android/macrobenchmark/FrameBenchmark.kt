package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.ExperimentalMetricApi
import androidx.benchmark.macro.FrameTimingGfxInfoMetric
import androidx.benchmark.macro.FrameTimingMetric
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
 * The fixed 60-second scripted frame round from section 6.
 *
 * FrameTimingMetric reports per-frame CPU presentation data and
 * FrameTimingGfxInfoMetric the gfxinfo percentiles; the shared journey loops
 * until the script window closes so scroll/animation demand runs the whole
 * round at the display's demanded cadence. run.py pairs these records with
 * the Perfetto frametimeline trace for surface-level coverage — Compose draws
 * through the decor window, but the gate asserts coverage exists regardless
 * of painter.
 *
 * Instrumentation args: fixture (default "suite"), script_seconds (60),
 * iterations (1).
 */
@OptIn(ExperimentalMetricApi::class)
@RunWith(androidx.test.ext.junit.runners.AndroidJUnit4::class)
class FrameBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    private val device: UiDevice =
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun frames() {
        val args = InstrumentationRegistry.getArguments()
        val fixture = args.getString("fixture") ?: "suite"
        val scriptSeconds = args.getString("script_seconds")?.toIntOrNull() ?: 60
        val iterations = args.getString("iterations")?.toIntOrNull() ?: 1
        val spec = loadInteractionSpec(
            InstrumentationRegistry.getInstrumentation().context,
        )
        benchmarkRule.measureRepeated(
            packageName = SuiteRegistry.packageForFixture(fixture),
            metrics = listOf(FrameTimingMetric(), FrameTimingGfxInfoMetric()),
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
                intent.putExtra("waterui.bench.markers", true)
            }
            device.wait(Until.hasObject(By.res("screen-root")), 10_000)
            Journey.runFor(device, spec, fixture, scriptSeconds)
        }
    }
}
