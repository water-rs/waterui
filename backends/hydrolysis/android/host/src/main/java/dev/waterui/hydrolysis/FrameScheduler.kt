package dev.waterui.hydrolysis

import android.util.Log
import android.view.Choreographer

/**
 * The frame pump: one Choreographer callback at a time drives one native frame
 * transaction. The transaction's outcome decides what happens next —
 *
 * - bit 1 (wants next frame): the engine has pending work → post again.
 * - bit 4 (deadline): a gesture/animation deadline exists → a delayed frame
 *   lands at that point in case no other wake arrives first.
 * - nothing set: the window is idle. No callbacks, no CPU or GPU work — an
 *   idle window never pumps.
 *
 * Every wake is logged at debug with its cause so an idle-CPU reading on a
 * device can attribute its jiffies: a frame nobody posted shows as
 * `cause=external`, anything else names the post that scheduled it.
 *
 * Refresh-rate demand is separate: while the scheduler is running continuous
 * frames the session asks the surface for its highest refresh; when the pump
 * goes idle the demand releases.
 */
internal class FrameScheduler(private val session: HydrolysisSession) :
    Choreographer.FrameCallback {

    private val choreographer = Choreographer.getInstance()
    private var posted = false
    private var deadlinePosted = false
    private var pumping = false

    /** Which post queued the pending callback; null when nobody did. */
    private var wakeCause: String? = null

    /**
     * Posts the next vsync frame if none is already queued. A pending
     * deadline frame is not a queued vsync frame: it is replaced, so the
     * request runs at the next vsync instead of waiting for the deadline.
     */
    fun requestFrame(cause: String = "redraw-request") {
        if (deadlinePosted) {
            choreographer.removeFrameCallback(this)
            deadlinePosted = false
        } else if (posted) {
            return
        }
        posted = true
        wakeCause = cause
        choreographer.postFrameCallback(this)
    }

    /** The host reports active interaction (touch held, animation running). */
    fun setInteractionActive(active: Boolean) {
        NativeBridge.nativeSetHighRefresh(session.nativePtr, if (active) -1f else 0f)
    }

    override fun doFrame(vsyncNanos: Long) {
        posted = false
        deadlinePosted = false
        val cause = wakeCause ?: "external"
        wakeCause = null
        val outcome = NativeBridge.nativeOnFrame(session.nativePtr, vsyncNanos)
        val wantsNext = outcome and WANTS_NEXT_FRAME != 0L
        val deadlineNanos =
            if (outcome and HAS_DEADLINE != 0L) {
                NativeBridge.nativeFrameDeadlineInNanos(session.nativePtr)
            } else {
                -1L
            }
        Log.d(
            TAG,
            "frame wake: cause=$cause outcome=0x${java.lang.Long.toHexString(outcome)} deadlineNs=$deadlineNanos",
        )

        if (wantsNext) {
            requestFrame("next-frame")
        } else if (deadlineNanos >= 0 && !posted) {
            // The engine's next wake is a deadline, not continuous work — a
            // single delayed frame covers it. An elapsed deadline returns -1
            // from the native side (its tick already ran inside this
            // transaction) and never reaches here. A vsync frame the
            // transaction itself requested already covers the deadline: it
            // runs first and reports the deadline again.
            deadlinePosted = true
            wakeCause = "deadline"
            choreographer.postFrameCallbackDelayed(
                this,
                deadlineNanos / NANOS_PER_MILLI,
            )
            posted = true
        }

        val nowPumping = wantsNext || posted
        if (nowPumping != pumping) {
            pumping = nowPumping
            // Continuous pumping asks for high refresh; an idle pump releases
            // the request so the panel can drop to its base rate.
            NativeBridge.nativeSetHighRefresh(session.nativePtr, if (pumping) -1f else 0f)
        }
    }

    private companion object {
        const val TAG = "hydrolysis"
        const val WANTS_NEXT_FRAME = 1L
        const val HAS_DEADLINE = 4L
        const val NANOS_PER_MILLI = 1_000_000L
    }
}
