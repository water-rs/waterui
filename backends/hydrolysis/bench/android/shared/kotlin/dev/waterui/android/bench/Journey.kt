package dev.waterui.android.bench

import android.os.SystemClock
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import org.json.JSONArray
import org.json.JSONObject

/**
 * The shared interaction-specification runner.
 *
 * bench/android/interactions/interactions.json is the single source for the scripts every
 * instrumented pass executes: the macrobenchmark startup, frame and energy tests, the
 * baseline-profile generator, and (via its own loader) run.py's memory and marker replays. Keeping
 * one spec keeps the journeys identical across instruments, which section 6 requires.
 *
 * Ops (one JSON object each): {"op": "wait_ms", "ms": 500} {"op": "settle"} wait for the window to
 * go idle {"op": "idle", "seconds": 10} idle-power segment of an energy run {"op": "tap",
 * <selector>} {"op": "long_click", <selector>} {"op": "input_text", <selector>, "text": "…"}
 * focus + commit text {"op": "ime_input", <selector>, "text": "…"} commit into focused field {"op":
 * "scroll", <selector>, "direction": "down|up|left|right", "times": 10} {"op": "swipe",
 * "direction": "…", "times": 3} full-screen fling {"op": "drag", "from": <selector>, "to":
 * <selector>} {"op": "wait_for", <selector>, "timeout_ms": 10000} {"op": "back"} {"op": "key",
 * "code": 4} {"op": "home"} {"op": "repeat", "times": 3, "steps": [ … ]}
 *
 * Selectors: {"tag": "…"} resolves a Compose testTag / view resource-id; {"text": "…"} and {"desc":
 * "…"} resolve exact content; {"clazz": "…"} resolves a class name.
 */
object Journey {

    /** Prepare immediately before an iteration; installation may outlast screen timeout. */
    fun prepareDevice(device: UiDevice) {
        device.wakeUp()
        device.executeShellCommand("wm dismiss-keyguard")
        device.executeShellCommand("cmd statusbar collapse")
    }

    fun loadSpec(assetText: String): JSONObject = JSONObject(assetText)

    /** Steps for `fixture` from a parsed spec; empty when it has none. */
    fun steps(spec: JSONObject, fixture: String): JSONArray {
        val fixtures = spec.getJSONObject("fixtures")
        if (!fixtures.has(fixture)) {
            throw IllegalArgumentException("no journey for fixture '$fixture' in interactions spec")
        }
        return fixtures.getJSONObject(fixture).getJSONArray("journey")
    }

    /** Run `steps` once. */
    fun run(device: UiDevice, spec: JSONObject, fixture: String) {
        runSteps(device, steps(spec, fixture))
    }

    /** A serial replay whose completion is owned outside the measured window. */
    class TimedRun(
        private val device: UiDevice,
        spec: JSONObject,
        fixture: String,
        private val completionTimeoutSeconds: Long,
    ) : AutoCloseable {
        private val journey = steps(spec, fixture)
        private var pending: FutureTask<Unit>? = null

        init {
            require(journey.length() > 0) { "timed journey must contain steps" }
            require(completionTimeoutSeconds > 0) { "replay completion timeout must be positive" }
        }

        /** Measure a fixed window; an in-flight gesture finishes outside that window. */
        fun measure(seconds: Int) {
            require(seconds > 0) { "journey duration must be positive" }
            check(pending == null) { "Complete the preceding replay in setup before measuring" }
            val deadline = SystemClock.elapsedRealtime() + seconds * 1000L
            val task =
                FutureTask<Unit> {
                    while (!expired(deadline)) runSteps(device, journey, deadline)
                }
            pending = task
            // One owned replay thread preserves action order; no parallel UI actions are submitted.
            Thread(task, "WaterUI benchmark journey").start()
            try {
                task.get(remaining(seconds * 1000L, deadline), TimeUnit.MILLISECONDS)
            } catch (_: TimeoutException) {
                // The measurement ends now. finish() observes completion/errors outside the trace.
            }
        }

        /**
         * Called before changing target state and during final cleanup, never inside measurement.
         */
        fun finish() {
            val task = pending ?: return
            try {
                task.get(completionTimeoutSeconds, TimeUnit.SECONDS)
            } catch (error: TimeoutException) {
                task.cancel(true)
                throw IllegalStateException(
                    "Replay did not finish after the measurement window",
                    error,
                )
            } finally {
                pending = null
            }
        }

        override fun close() = finish()
    }

    /** A fixed idle measurement window without a replay task. */
    fun idleFor(seconds: Int) {
        require(seconds > 0) { "idle duration must be positive" }
        val duration = seconds * 1000L
        val deadline = SystemClock.elapsedRealtime() + duration
        while (!expired(deadline)) pause(duration, deadline)
    }

    private fun selector(node: JSONObject): BySelector? {
        val sel = node.optJSONObject("selector") ?: node
        return when {
            sel.has("tag") -> By.res(sel.getString("tag"))
            sel.has("text") -> By.text(sel.getString("text"))
            sel.has("desc") -> By.desc(sel.getString("desc"))
            sel.has("clazz") -> By.clazz(sel.getString("clazz"))
            else -> null
        }
    }

    private fun direction(name: String): Direction =
        when (name.lowercase()) {
            "down",
            "forward" -> Direction.DOWN
            "up",
            "backward" -> Direction.UP
            "left" -> Direction.LEFT
            "right" -> Direction.RIGHT
            else -> throw IllegalArgumentException("unknown direction '$name'")
        }

    private fun swipeDirection(device: UiDevice, name: String, times: Int, deadline: Long?) {
        val w = device.displayWidth
        val h = device.displayHeight
        val cx = w / 2
        val cy = h / 2
        repeat(times) {
            if (expired(deadline)) return
            when (name.lowercase()) {
                "up" -> device.swipe(cx, (h * 0.8).toInt(), cx, (h * 0.2).toInt(), 12)
                "down" -> device.swipe(cx, (h * 0.2).toInt(), cx, (h * 0.8).toInt(), 12)
                "left" -> device.swipe((w * 0.8).toInt(), cy, (w * 0.2).toInt(), cy, 12)
                "right" -> device.swipe((w * 0.2).toInt(), cy, (w * 0.8).toInt(), cy, 12)
                else -> throw IllegalArgumentException("unknown direction '$name'")
            }
            waitForIdle(device, 500, deadline)
        }
    }

    private fun expired(deadline: Long?): Boolean =
        deadline != null && SystemClock.elapsedRealtime() >= deadline

    private fun remaining(timeout: Long, deadline: Long?): Long =
        if (deadline == null) timeout
        else minOf(timeout, (deadline - SystemClock.elapsedRealtime()).coerceAtLeast(0))

    private fun waitForIdle(device: UiDevice, timeout: Long, deadline: Long?) {
        val duration = remaining(timeout, deadline)
        // Do not enter an idle wait after the measurement window has closed.
        if (duration > 0) device.waitForIdle(duration)
    }

    private fun pause(duration: Long, deadline: Long?) {
        val durationLeft = remaining(duration, deadline)
        if (durationLeft > 0) Thread.sleep(durationLeft)
    }

    private fun runSteps(device: UiDevice, steps: JSONArray, deadline: Long? = null) {
        for (i in 0 until steps.length()) {
            if (expired(deadline)) return
            val step = steps.getJSONObject(i)
            when (val op = step.getString("op")) {
                "wait_ms" -> pause(step.getLong("ms"), deadline)
                "settle" -> waitForIdle(device, 5000, deadline)
                "idle" -> pause(step.getLong("seconds") * 1000, deadline)
                "tap" -> {
                    val sel = requireNotNull(selector(step)) { "tap needs a selector" }
                    val obj =
                        device.wait(Until.findObject(sel), remaining(10_000, deadline))
                            ?: run {
                                if (expired(deadline)) return
                                throw IllegalStateException("tap: no object for $sel")
                            }
                    if (expired(deadline)) return
                    obj.click()
                    waitForIdle(device, 500, deadline)
                }
                "long_click" -> {
                    val sel = requireNotNull(selector(step)) { "long_click needs a selector" }
                    val obj =
                        device.wait(Until.findObject(sel), remaining(10_000, deadline))
                            ?: run {
                                if (expired(deadline)) return
                                throw IllegalStateException("long_click: no object for $sel")
                            }
                    if (expired(deadline)) return
                    obj.longClick()
                    waitForIdle(device, 500, deadline)
                }
                "input_text",
                "ime_input" -> {
                    val sel = selector(step)
                    val obj =
                        if (sel != null) {
                            device.wait(Until.findObject(sel), remaining(10_000, deadline))
                        } else {
                            device.findObject(By.focused(true))
                        }
                            ?: run {
                                if (expired(deadline)) return
                                throw IllegalStateException("input: no field for $sel")
                            }
                    if (expired(deadline)) return
                    obj.click()
                    obj.setText(step.getString("text"))
                    waitForIdle(device, 500, deadline)
                }
                "scroll" -> {
                    val sel = selector(step)
                    val scrollable =
                        if (sel != null) {
                            device.wait(Until.findObject(sel), remaining(10_000, deadline))
                        } else {
                            device.findObject(By.scrollable(true))
                        }
                            ?: run {
                                if (expired(deadline)) return
                                throw IllegalStateException("scroll: no scrollable for $sel")
                            }
                    val dir = direction(step.getString("direction"))
                    repeat(step.optInt("times", 10)) {
                        if (expired(deadline)) return
                        scrollable.scroll(dir, 1.0f, 2000)
                        waitForIdle(device, 300, deadline)
                    }
                }
                "swipe" ->
                    swipeDirection(
                        device,
                        step.getString("direction"),
                        step.optInt("times", 1),
                        deadline,
                    )
                "drag" -> {
                    val from = requireNotNull(selector(step.getJSONObject("from")))
                    val to = requireNotNull(selector(step.getJSONObject("to")))
                    val a =
                        device.wait(Until.findObject(from), remaining(10_000, deadline))
                            ?: run {
                                if (expired(deadline)) return
                                throw IllegalStateException("drag: no source for $from")
                            }
                    val b =
                        device.wait(Until.findObject(to), remaining(10_000, deadline))
                            ?: run {
                                if (expired(deadline)) return
                                throw IllegalStateException("drag: no target for $to")
                            }
                    if (expired(deadline)) return
                    val ab = a.visibleBounds
                    val bb = b.visibleBounds
                    device.drag(
                        ab.centerX(),
                        ab.centerY(),
                        bb.centerX(),
                        bb.centerY(),
                        60,
                    )
                    waitForIdle(device, 500, deadline)
                }
                "wait_for" -> {
                    val sel = requireNotNull(selector(step)) { "wait_for needs a selector" }
                    val found =
                        device.wait(
                            Until.hasObject(sel),
                            remaining(step.optLong("timeout_ms", 10_000), deadline),
                        )
                    if (expired(deadline)) return
                    check(found) { "wait_for: nothing matched $sel" }
                }
                "back" -> device.pressBack()
                "key" -> device.pressKeyCode(step.getInt("code"))
                "home" -> device.pressHome()
                "repeat" ->
                    repeat(step.optInt("times", 1)) {
                        runSteps(device, step.getJSONArray("steps"), deadline)
                    }
                else -> throw IllegalArgumentException("unknown op '$op'")
            }
        }
    }
}
