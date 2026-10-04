package dev.waterui.android.bench

import androidx.test.uiautomator.By
import androidx.test.uiautomator.BySelector
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.Until
import org.json.JSONArray
import org.json.JSONObject

/**
 * The shared interaction-specification runner.
 *
 * bench/android/interactions/interactions.json is the single source for the
 * scripts every instrumented pass executes: the macrobenchmark startup,
 * frame and energy tests, the baseline-profile generator, and (via its own
 * loader) run.py's memory and marker replays. Keeping one spec keeps the
 * journeys identical across instruments, which section 6 requires.
 *
 * Ops (one JSON object each):
 *   {"op": "wait_ms", "ms": 500}
 *   {"op": "settle"}                      wait for the window to go idle
 *   {"op": "idle", "seconds": 10}         idle-power segment of an energy run
 *   {"op": "tap",      <selector>}
 *   {"op": "long_click", <selector>}
 *   {"op": "input_text", <selector>, "text": "…"}   focus + commit text
 *   {"op": "ime_input", <selector>, "text": "…"}    commit into focused field
 *   {"op": "scroll", <selector>, "direction": "down|up|left|right",
 *    "times": 10}
 *   {"op": "swipe",  "direction": "…", "times": 3}  full-screen fling
 *   {"op": "drag",   "from": <selector>, "to": <selector>}
 *   {"op": "wait_for", <selector>, "timeout_ms": 10000}
 *   {"op": "back"}
 *   {"op": "key", "code": 4}
 *   {"op": "home"}
 *   {"op": "repeat", "times": 3, "steps": [ … ]}
 *
 * Selectors: {"tag": "…"} resolves a Compose testTag / view resource-id;
 * {"text": "…"} and {"desc": "…"} resolve exact content; {"clazz": "…"}
 * resolves a class name.
 */
object Journey {

    fun loadSpec(assetText: String): JSONObject = JSONObject(assetText)

    /** Steps for `fixture` from a parsed spec; empty when it has none. */
    fun steps(spec: JSONObject, fixture: String): JSONArray {
        val fixtures = spec.getJSONObject("fixtures")
        if (!fixtures.has(fixture)) {
            throw IllegalArgumentException(
                "no journey for fixture '$fixture' in interactions spec"
            )
        }
        return fixtures.getJSONObject(fixture).getJSONArray("journey")
    }

    /** Run `steps` once. */
    fun run(device: UiDevice, spec: JSONObject, fixture: String) {
        runSteps(device, steps(spec, fixture))
    }

    /**
     * Run the fixture's journey repeatedly until `seconds` have elapsed —
     * the fixed 60-second script a frame/energy round measures.
     */
    fun runFor(device: UiDevice, spec: JSONObject, fixture: String, seconds: Int) {
        val deadline = System.currentTimeMillis() + seconds * 1000L
        val journey = steps(spec, fixture)
        do {
            runSteps(device, journey)
        } while (System.currentTimeMillis() < deadline)
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
            "down", "forward" -> Direction.DOWN
            "up", "backward" -> Direction.UP
            "left" -> Direction.LEFT
            "right" -> Direction.RIGHT
            else -> throw IllegalArgumentException("unknown direction '$name'")
        }

    private fun swipeDirection(device: UiDevice, name: String, times: Int) {
        val w = device.displayWidth
        val h = device.displayHeight
        val cx = w / 2
        val cy = h / 2
        repeat(times) {
            when (name.lowercase()) {
                "up" -> device.swipe(cx, (h * 0.8).toInt(), cx, (h * 0.2).toInt(), 12)
                "down" -> device.swipe(cx, (h * 0.2).toInt(), cx, (h * 0.8).toInt(), 12)
                "left" -> device.swipe((w * 0.8).toInt(), cy, (w * 0.2).toInt(), cy, 12)
                "right" -> device.swipe((w * 0.2).toInt(), cy, (w * 0.8).toInt(), cy, 12)
                else -> throw IllegalArgumentException("unknown direction '$name'")
            }
            device.waitForIdle(500)
        }
    }

    private fun runSteps(device: UiDevice, steps: JSONArray) {
        for (i in 0 until steps.length()) {
            val step = steps.getJSONObject(i)
            when (val op = step.getString("op")) {
                "wait_ms" -> Thread.sleep(step.getLong("ms"))
                "settle" -> device.waitForIdle(5000)
                "idle" -> Thread.sleep(step.getLong("seconds") * 1000)
                "tap" -> {
                    val sel = requireNotNull(selector(step)) { "tap needs a selector" }
                    val obj = device.wait(Until.findObject(sel), 10_000)
                        ?: throw IllegalStateException("tap: no object for $sel")
                    obj.click()
                    device.waitForIdle(500)
                }
                "long_click" -> {
                    val sel = requireNotNull(selector(step)) { "long_click needs a selector" }
                    val obj = device.wait(Until.findObject(sel), 10_000)
                        ?: throw IllegalStateException("long_click: no object for $sel")
                    obj.longClick()
                    device.waitForIdle(500)
                }
                "input_text", "ime_input" -> {
                    val sel = selector(step)
                    val obj = if (sel != null) {
                        device.wait(Until.findObject(sel), 10_000)
                    } else {
                        device.findObject(By.focused(true))
                    } ?: throw IllegalStateException("input: no field for $sel")
                    obj.click()
                    obj.setText(step.getString("text"))
                    device.waitForIdle(500)
                }
                "scroll" -> {
                    val sel = selector(step)
                    val scrollable = if (sel != null) {
                        device.wait(Until.findObject(sel), 10_000)
                    } else {
                        device.findObject(By.scrollable(true))
                    } ?: throw IllegalStateException("scroll: no scrollable for $sel")
                    val dir = direction(step.getString("direction"))
                    repeat(step.optInt("times", 10)) {
                        scrollable.scroll(dir, 1.0f, 2000)
                        device.waitForIdle(300)
                    }
                }
                "swipe" -> swipeDirection(
                    device,
                    step.getString("direction"),
                    step.optInt("times", 1),
                )
                "drag" -> {
                    val from = requireNotNull(selector(step.getJSONObject("from")))
                    val to = requireNotNull(selector(step.getJSONObject("to")))
                    val a = device.wait(Until.findObject(from), 10_000)
                        ?: throw IllegalStateException("drag: no source for $from")
                    val b = device.wait(Until.findObject(to), 10_000)
                        ?: throw IllegalStateException("drag: no target for $to")
                    val ab = a.visibleBounds
                    val bb = b.visibleBounds
                    device.drag(
                        ab.centerX(), ab.centerY(),
                        bb.centerX(), bb.centerY(), 60,
                    )
                    device.waitForIdle(500)
                }
                "wait_for" -> {
                    val sel = requireNotNull(selector(step)) { "wait_for needs a selector" }
                    val found = device.wait(
                        Until.hasObject(sel),
                        step.optLong("timeout_ms", 10_000),
                    )
                    check(found) { "wait_for: nothing matched $sel" }
                }
                "back" -> device.pressBack()
                "key" -> device.pressKeyCode(step.getInt("code"))
                "home" -> device.pressHome()
                "repeat" -> repeat(step.optInt("times", 1)) {
                    runSteps(device, step.getJSONArray("steps"))
                }
                else -> throw IllegalArgumentException("unknown op '$op'")
            }
        }
    }
}
