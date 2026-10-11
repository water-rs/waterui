package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.BaselineProfileMode
import androidx.benchmark.macro.CompilationMode
import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.StartupTimingMetric
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import dev.waterui.android.bench.SuiteRegistry
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Launch-time measurement per section 6.
 *
 * COLD is ordinary cold-process startup; WARM is Activity creation inside an
 * existing process (the benchmark recreates the activity, never foregrounds
 * an already-created one); `firstInstall` is the separate first-launch
 * measurement — `pm clear` gives it a cache-empty process before the single
 * cold start. Initial display is gated on the twin's root landing on screen
 * (`screen-root` testTag), so a blank host never counts as content.
 *
 * Fixture and sample count come from instrumentation args so run.py drives
 * the same test for every fixture in the frozen inventory:
 *   fixture=<suite.toml name>   (default "suite")
 *   iterations=<n>              (default 30)
 */
@RunWith(androidx.test.ext.junit.runners.AndroidJUnit4::class)
class StartupBenchmark {

    @get:Rule
    val benchmarkRule = MacrobenchmarkRule()

    private val device: UiDevice =
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    private fun args() = InstrumentationRegistry.getArguments()
    private fun fixture() = args().getString("fixture") ?: "suite"
    private fun iterations() = args().getString("iterations")?.toIntOrNull() ?: 30

    private fun androidx.benchmark.macro.MacrobenchmarkScope.launchWithSpecEnv(
        fixture: String,
    ) {
        startActivityAndWait { intent ->
            intent.putExtra("E2EExample", SuiteRegistry.exampleForFixture(fixture))
            intent.putExtra("waterui.env.WATERUI_DISABLE_DYNAMIC_COLORS", "1")
            intent.putExtra("waterui.bench.markers", true)
        }
        // First meaningful content, not the splash/blank host.
        device.wait(Until.hasObject(By.res("screen-root")), 10_000)
    }

    @Test
    fun cold() = startup(StartupMode.COLD)

    @Test
    fun warm() = startup(StartupMode.WARM)

    @Test
    fun firstInstall() {
        val fixture = fixture()
        val pkg = SuiteRegistry.packageForFixture(fixture)
        // First-install/cache-empty: wipe app data so the next cold start is
        // the first launch a fresh install sees, then take one cold sample.
        device.executeShellCommand("pm clear $pkg")
        benchmarkRule.measureRepeated(
            packageName = pkg,
            metrics = listOf(StartupTimingMetric()),
            compilationMode = CompilationMode.Partial(
                BaselineProfileMode.Require,
            ),
            iterations = 1,
            startupMode = StartupMode.COLD,
            setupBlock = { pressHome() },
        ) {
            launchWithSpecEnv(fixture)
        }
    }

    private fun startup(mode: StartupMode) {
        val fixture = fixture()
        benchmarkRule.measureRepeated(
            packageName = SuiteRegistry.packageForFixture(fixture),
            metrics = listOf(StartupTimingMetric()),
            compilationMode = CompilationMode.Partial(
                BaselineProfileMode.Require,
            ),
            iterations = iterations(),
            startupMode = mode,
            setupBlock = {
                pressHome()
                if (mode == StartupMode.WARM) {
                    // Warm = Activity creation in a live process: ensure the
                    // process exists, then leave the app backgrounded so the
                    // measured launch creates the Activity, not the process.
                    launchWithSpecEnv(fixture)
                    pressHome()
                }
            },
        ) {
            launchWithSpecEnv(fixture)
        }
    }
}
