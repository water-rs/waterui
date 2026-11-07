package dev.waterui.hydrolysis.testapp

import android.view.View
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.gpu.HydrolysisGpuBand
import dev.waterui.hydrolysis.gpu.HydrolysisGpuTopBand

/** The GPU test app: Cherenkov paints through the bands beneath and above the host. */
class MainActivity : TestActivity() {
    override fun createContentView(session: HydrolysisSession): View =
        createHost(session).apply {
            addView(HydrolysisGpuBand(this@MainActivity, session), 0)
            // Above the platform-view overlay: entries a frame paints over its
            // embedded views present into the top band (water-rs/waterui#2225).
            addView(HydrolysisGpuTopBand(this@MainActivity, session))
        }
}
