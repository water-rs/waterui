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
            // Each embedded platform view gets the band its above-mark
            // entries present onto: the registry interleaves a band after
            // every ordered platform view (water-rs/waterui#2225).
            platformViewRegistry.bandFactory = { region ->
                HydrolysisGpuTopBand(this@MainActivity, session, region)
            }
        }
}
