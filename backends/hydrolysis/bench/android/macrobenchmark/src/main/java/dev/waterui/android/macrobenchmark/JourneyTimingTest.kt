package dev.waterui.android.macrobenchmark

import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import dev.waterui.android.bench.Journey
import java.util.concurrent.ExecutionException
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(androidx.test.ext.junit.runners.AndroidJUnit4::class)
class JourneyTimingTest {
    private val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())

    private fun spec(step: JSONObject): JSONObject =
        JSONObject()
            .put(
                "fixtures",
                JSONObject().put("test", JSONObject().put("journey", JSONArray().put(step))),
            )

    @Test
    fun replayFailureReachesMeasurementAndCleanup() {
        val replay =
            Journey.TimedRun(device, spec(JSONObject().put("op", "invalid-test-op")), "test", 2)
        val measurement = assertThrows(ExecutionException::class.java) { replay.measure(1) }
        assertTrue(measurement.cause is IllegalArgumentException)
        val cleanup = assertThrows(ExecutionException::class.java) { replay.finish() }
        assertTrue(cleanup.cause is IllegalArgumentException)
        replay.close()
    }

    @Test
    fun nextWindowRequiresExplicitCompletion() {
        Journey.TimedRun(device, spec(JSONObject().put("op", "wait_ms").put("ms", 10)), "test", 2)
            .use { replay ->
                replay.measure(1)
                assertThrows(IllegalStateException::class.java) { replay.measure(1) }
                replay.finish()
            }
    }
}
