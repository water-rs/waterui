package dev.waterui.hydrolysis

import android.content.Context

/**
 * Implemented by platform-view instances that keep Activity-scoped state —
 * the system WebView's context wrapper and `Lifecycle` observer — across
 * host-view rebindings such as a configuration change.
 * [HydrolysisSession.bind] calls it on every registered instance.
 */
interface HostRebindAware {
    /** A new host view bound with `context`; re-observe what it owns. */
    fun onHostRebound(context: Context)
}
