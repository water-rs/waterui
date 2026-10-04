package dev.waterui.android.reference

import android.os.SystemClock
import android.util.Log
import android.view.Choreographer
import java.util.concurrent.atomic.AtomicInteger

/**
 * Input-latency measurement markers, per section 6.
 *
 * When the app is launched with `waterui.bench.markers=true`, interactive
 * twins emit a matched pair of logcat lines around each measured gesture:
 *
 *   WUI-BENCH-INPUT   id=<n> kind=<tap|ime_edit> t_input=<uptime_ms>
 *   WUI-BENCH-PRESENT id=<n> t_present=<uptime_ms>
 *
 * INPUT is logged at gesture delivery; PRESENT is logged inside the next
 * Choreographer frame after the state change the gesture caused — the first
 * presented frame containing its result, not "time until invalidate".
 * metrics/input_latency.py pairs them per id.
 *
 * The markers sit behind the same benchmark flag the Hydrolysis host reads,
 * so both sides measure with identical overhead; the flag is off in ordinary
 * runs.
 */
object BenchMarkers {
    @Volatile var enabled: Boolean = false

    private val nextId = AtomicInteger(0)

    fun input(kind: String): Int {
        if (!enabled) return -1
        val id = nextId.incrementAndGet()
        Log.i(
            "WUI-BENCH-INPUT",
            "id=$id kind=$kind t_input=${SystemClock.uptimeMillis()}",
        )
        return id
    }

    fun present(id: Int) {
        if (id < 0) return
        Choreographer.getInstance().postFrameCallback { frameNs ->
            Log.i(
                "WUI-BENCH-PRESENT",
                "id=$id t_present=${frameNs / 1_000_000}",
            )
        }
    }

    /** One tap-to-present trial. */
    fun tap() {
        present(input("tap"))
    }

    /** One IME-edit-to-present trial. */
    fun edit() {
        present(input("ime_edit"))
    }
}
