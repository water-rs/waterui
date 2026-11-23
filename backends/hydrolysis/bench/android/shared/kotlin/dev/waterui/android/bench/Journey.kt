package dev.waterui.android.bench

import android.graphics.Rect
import android.os.SystemClock
import android.os.Trace
import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Configurator
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicInteger
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
 * Inside `TimedRun` (`paced = true`), steps advance on monotonic `elapsedRealtime()` budgets
 * instead of target-dependent `UiDevice.waitForIdle(...)`, so Compose and Hydrolysis execute equal
 * interaction counts within an equal-duration measurement window. Every dispatched input action
 * records a `"journeyAction"` trace section for `TraceSectionMetric` verification.
 */
object Journey {
    private const val PACED_SETTLE_MS = 250L
    private const val PACED_SCROLL_PERIOD_MS = 400L
    private const val PACED_SWIPE_PERIOD_MS = 600L
    private const val PACED_ACTION_PERIOD_MS = 600L
    private const val SWIPE_STEPS = 12

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
        private val completedActions = AtomicInteger(0)
        private var savedWaitForIdleTimeout: Long? = null
        private var savedActionAckTimeout: Long? = null

        init {
            require(journey.length() > 0) { "timed journey must contain steps" }
            require(completionTimeoutSeconds > 0) { "replay completion timeout must be positive" }
        }

        /** Completed input actions recorded during the most recent `measure` window. */
        fun lastActionCount(): Int = completedActions.get()

        /** Measure a fixed window; an in-flight gesture finishes outside that window. */
        fun measure(seconds: Int) {
            require(seconds > 0) { "journey duration must be positive" }
            check(pending == null) { "Complete the preceding replay in setup before measuring" }
            completedActions.set(0)
            val configurator = Configurator.getInstance()
            savedWaitForIdleTimeout = configurator.waitForIdleTimeout
            savedActionAckTimeout = configurator.actionAcknowledgmentTimeout
            configurator.waitForIdleTimeout = 0L
            configurator.actionAcknowledgmentTimeout = 0L
            val deadline = SystemClock.elapsedRealtime() + seconds * 1000L
            val task =
                FutureTask<Unit> {
                    while (!expired(deadline)) {
                        runSteps(device, journey, deadline, paced = true, actions = completedActions)
                    }
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
            val task = pending
            try {
                if (task != null) {
                    try {
                        task.get(completionTimeoutSeconds, TimeUnit.SECONDS)
                    } catch (error: TimeoutException) {
                        task.cancel(true)
                        throw IllegalStateException(
                            "Replay did not finish after the measurement window",
                            error,
                        )
                    }
                }
            } finally {
                pending = null
                val configurator = Configurator.getInstance()
                savedWaitForIdleTimeout?.let { configurator.waitForIdleTimeout = it }
                savedActionAckTimeout?.let { configurator.actionAcknowledgmentTimeout = it }
                savedWaitForIdleTimeout = null
                savedActionAckTimeout = null
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

    private inline fun tracedAction(actions: AtomicInteger?, block: () -> Unit) {
        Trace.beginSection("journeyAction")
        try {
            block()
        } finally {
            Trace.endSection()
        }
        actions?.incrementAndGet()
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

    private fun swipeBounds(device: UiDevice, bounds: Rect, fingerDirection: String) {
        val cx = bounds.centerX()
        val cy = bounds.centerY()
        val x20 = bounds.left + (bounds.width() * 0.2).toInt()
        val x80 = bounds.left + (bounds.width() * 0.8).toInt()
        val y20 = bounds.top + (bounds.height() * 0.2).toInt()
        val y80 = bounds.top + (bounds.height() * 0.8).toInt()
        when (fingerDirection.lowercase()) {
            "up" -> device.swipe(cx, y80, cx, y20, SWIPE_STEPS)
            "down" -> device.swipe(cx, y20, cx, y80, SWIPE_STEPS)
            "left" -> device.swipe(x80, cy, x20, cy, SWIPE_STEPS)
            "right" -> device.swipe(x20, cy, x80, cy, SWIPE_STEPS)
            else -> throw IllegalArgumentException("unknown direction '$fingerDirection'")
        }
    }

    private fun scrollFingerDirection(contentDirection: String): String =
        when (contentDirection.lowercase()) {
            "down",
            "forward" -> "up"
            "up",
            "backward" -> "down"
            "left" -> "right"
            "right" -> "left"
            else -> throw IllegalArgumentException("unknown direction '$contentDirection'")
        }

    private fun swipeDirection(
        device: UiDevice,
        name: String,
        times: Int,
        deadline: Long?,
        paced: Boolean,
        actions: AtomicInteger?,
    ) {
        val bounds = Rect(0, 0, device.displayWidth, device.displayHeight)
        repeat(times) {
            if (expired(deadline)) return
            val stepStart = SystemClock.elapsedRealtime()
            tracedAction(actions) { swipeBounds(device, bounds, name) }
            afterAction(device, stepStart, 500, PACED_SWIPE_PERIOD_MS, deadline, paced)
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

    private fun pauseUntil(stepTarget: Long, deadline: Long?) {
        val now = SystemClock.elapsedRealtime()
        if (stepTarget > now) {
            pause(stepTarget - now, deadline)
        }
    }

    private fun afterAction(
        device: UiDevice,
        stepStart: Long,
        idleTimeoutMs: Long,
        pacedPeriodMs: Long,
        deadline: Long?,
        paced: Boolean,
    ) {
        if (paced) {
            pauseUntil(stepStart + pacedPeriodMs, deadline)
        } else {
            waitForIdle(device, idleTimeoutMs, deadline)
        }
    }

    private fun runSteps(
        device: UiDevice,
        steps: JSONArray,
        deadline: Long? = null,
        paced: Boolean = false,
        actions: AtomicInteger? = null,
    ) {
        for (i in 0 until steps.length()) {
            if (expired(deadline)) return
            val step = steps.getJSONObject(i)
            when (val op = step.getString("op")) {
                "wait_ms" -> pause(step.getLong("ms"), deadline)
                "settle" ->
                    if (paced) {
                        pause(PACED_SETTLE_MS, deadline)
                    } else {
                        waitForIdle(device, 5000, deadline)
                    }
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
                    val stepStart = SystemClock.elapsedRealtime()
                    tracedAction(actions) { obj.click() }
                    afterAction(device, stepStart, 500, PACED_ACTION_PERIOD_MS, deadline, paced)
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
                    val stepStart = SystemClock.elapsedRealtime()
                    tracedAction(actions) { obj.longClick() }
                    afterAction(device, stepStart, 500, PACED_ACTION_PERIOD_MS, deadline, paced)
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
                    val stepStart = SystemClock.elapsedRealtime()
                    tracedAction(actions) {
                        obj.click()
                        obj.setText(step.getString("text"))
                    }
                    afterAction(device, stepStart, 500, PACED_ACTION_PERIOD_MS, deadline, paced)
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
                    if (paced) {
                        val bounds = scrollable.visibleBounds
                        val fingerDir = scrollFingerDirection(step.getString("direction"))
                        repeat(step.optInt("times", 10)) {
                            if (expired(deadline)) return
                            val stepStart = SystemClock.elapsedRealtime()
                            tracedAction(actions) { swipeBounds(device, bounds, fingerDir) }
                            pauseUntil(stepStart + PACED_SCROLL_PERIOD_MS, deadline)
                        }
                    } else {
                        val dir = direction(step.getString("direction"))
                        repeat(step.optInt("times", 10)) {
                            if (expired(deadline)) return
                            tracedAction(actions) { scrollable.scroll(dir, 1.0f, 2000) }
                            waitForIdle(device, 300, deadline)
                        }
                    }
                }
                "swipe" ->
                    swipeDirection(
                        device,
                        step.getString("direction"),
                        step.optInt("times", 1),
                        deadline,
                        paced,
                        actions,
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
                    val stepStart = SystemClock.elapsedRealtime()
                    tracedAction(actions) {
                        device.drag(
                            ab.centerX(),
                            ab.centerY(),
                            bb.centerX(),
                            bb.centerY(),
                            60,
                        )
                    }
                    afterAction(device, stepStart, 500, PACED_ACTION_PERIOD_MS, deadline, paced)
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
                "back" -> tracedAction(actions) { device.pressBack() }
                "key" -> tracedAction(actions) { device.pressKeyCode(step.getInt("code")) }
                "home" -> tracedAction(actions) { device.pressHome() }
                "repeat" ->
                    repeat(step.optInt("times", 1)) {
                        runSteps(device, step.getJSONArray("steps"), deadline, paced, actions)
                    }
                else -> throw IllegalArgumentException("unknown op '$op'")
            }
        }
    }
}
