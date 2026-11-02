package dev.waterui.hydrolysis.testapp

import android.view.View
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.gpu.HydrolysisGpuBand

/** The GPU test app: Cherenkov paints through the band beneath the host. */
class MainActivity : TestActivity() {
    override fun createContentView(session: HydrolysisSession): View =
        createHost(session).apply {
            addView(HydrolysisGpuBand(this@MainActivity, session), 0)
        }
}
