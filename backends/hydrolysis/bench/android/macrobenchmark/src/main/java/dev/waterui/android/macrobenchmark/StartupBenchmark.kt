package dev.waterui.android.macrobenchmark

import androidx.benchmark.macro.StartupMode
import androidx.benchmark.macro.StartupTimingMetric
import androidx.benchmark.macro.junit4.MacrobenchmarkRule
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import dev.waterui.android.bench.Journey
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
        target: BenchmarkTarget,
    ) {
        startActivityAndWait { intent ->
            intent.putExtra("E2EExample", SuiteRegistry.exampleForFixture(fixture))
            intent.putExtra("waterui.env.WATERUI_DISABLE_DYNAMIC_COLORS", "1")
            intent.putExtra("waterui.bench.markers", true)
        }
        // First meaningful content, not the splash/blank host.
        target.awaitContent(device)
    }

    @Test
    fun cold() = startup(StartupMode.COLD)

    @Test
    fun warm() = startup(StartupMode.WARM)

    @Test
    fun firstInstall() {
        val fixture = fixture()
        val target = BenchmarkTarget(args(), fixture)
        val pkg = target.packageName
        // First-install/cache-empty: wipe app data so the next cold start is
        // the first launch a fresh install sees, then take one cold sample.
        device.executeShellCommand("pm clear $pkg")
        benchmarkRule.measureRepeated(
            packageName = pkg,
            metrics = listOf(StartupTimingMetric()),
            compilationMode = target.compilationMode,
            iterations = 1,
            startupMode = StartupMode.COLD,
            setupBlock = {
                Journey.prepareDevice(device)
                pressHome()
            },
        ) {
            launchWithSpecEnv(fixture, target)
        }
    }

    private fun startup(mode: StartupMode) {
        val fixture = fixture()
        val target = BenchmarkTarget(args(), fixture)
        benchmarkRule.measureRepeated(
            packageName = target.packageName,
            metrics = listOf(StartupTimingMetric()),
            compilationMode = target.compilationMode,
            iterations = iterations(),
            startupMode = mode,
            setupBlock = {
                Journey.prepareDevice(device)
                pressHome()
                if (mode == StartupMode.WARM) {
                    // Warm = Activity creation in a live process: ensure the
                    // process exists, then leave the app backgrounded so the
                    // measured launch creates the Activity, not the process.
                    launchWithSpecEnv(fixture, target)
                    pressHome()
                }
            },
        ) {
            launchWithSpecEnv(fixture, target)
        }
    }
}
