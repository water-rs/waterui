package dev.waterui.android.baselineprofile

import androidx.benchmark.macro.junit4.BaselineProfileRule
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
 * Generates the baseline profile for the target-app variant the
 * androidx.baselineprofile plugin is generating (it injects `targetAppId`
 * per variant; this test build binds the suite variant). The block walks
 * EVERY fixture journey in the shared interaction spec against the suite
 * application — the suite APK carries all twins, so the merged profile
 * covers every screen the campaign measures. run.py `build` then copies the
 * generated `suiteRelease` profile into each per-fixture variant's profile
 * source, which is honest: each variant's APK ships the same classes for
 * its own twin plus the shared app scaffolding, and ART ignores profile
 * entries for classes a variant's R8 pass removed.
 */
@RunWith(androidx.test.ext.junit.runners.AndroidJUnit4::class)
class BaselineProfileGenerator {

    @get:Rule
    val baselineProfileRule = BaselineProfileRule()

    private val device: UiDevice =
        UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    @Test
    fun generate() {
        val args = InstrumentationRegistry.getArguments()
        val packageName = args.getString("targetAppId")
            ?: throw IllegalArgumentException(
                "targetAppId not passed as instrumentation runner arg"
            )
        val spec = loadInteractionSpec(
            InstrumentationRegistry.getInstrumentation().context,
        )
        val fixtures = spec.getJSONObject("fixtures").keys().asSequence()
            .toList()
            .sorted()
        baselineProfileRule.collect(
            packageName = packageName,
            includeInStartupProfile = true,
        ) {
            pressHome()
            for (fixture in fixtures) {
                android.util.Log.i("BaselineProfile", "journey: $fixture")
                try {
                    // The same launch path run.py uses: am start with the
                    // suite's E2EExample/env extras.
                    device.executeShellCommand(
                        "am start -W -n $packageName/.MainActivity " +
                            "--es E2EExample " +
                            SuiteRegistry.exampleForFixture(fixture) +
                            " --es waterui.env.WATERUI_DISABLE_DYNAMIC_COLORS 1",
                    )
                    device.wait(Until.hasObject(By.res("screen-root")), 10_000)
                    Journey.run(device, spec, fixture)
                } finally {
                    device.pressHome()
                    device.executeShellCommand("am force-stop $packageName")
                }
            }
        }
    }
}
