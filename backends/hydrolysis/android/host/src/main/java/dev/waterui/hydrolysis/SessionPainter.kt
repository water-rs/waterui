package dev.waterui.hydrolysis

/**
 * A session-owned painter the host view binds to: the painter outlives
 * view recreation, attaches to each new [HydrolysisHostView], and releases
 * only on final session teardown.
 */
interface SessionPainter {
    /** Binds this painter to `host`, invalidating it for the next frame. */
    fun attach(host: HydrolysisHostView)

    /** Unbinds `host`; a stale host's detach is a no-op. */
    fun detach(host: HydrolysisHostView)

    /** Final teardown; the painter must be detached first. */
    fun destroy()

    /** Applies or releases the high-refresh demand for the bound host. */
    fun setHighRefresh(active: Boolean)
}
