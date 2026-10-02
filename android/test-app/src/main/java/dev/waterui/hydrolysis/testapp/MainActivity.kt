package dev.waterui.hydrolysis.testapp

import android.view.View
import dev.waterui.hydrolysis.HydrolysisActivity
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.gpu.HydrolysisGpuBand

/**
 * The debug app exercising the Hydrolysis host end to end: the WaterUI tree
 * from `libhydrolysis_test_app.so` mounts on a [HydrolysisHostView] with the
 * GPU band as its bottom child.
 */
class MainActivity : HydrolysisActivity() {

    override val nativeLibraryName: String = "hydrolysis_test_app"

    override fun createContentView(session: HydrolysisSession): View {
        val host = HydrolysisHostView(this, session)
        // The GPU band is the bottom-most child; the platform-view overlay
        // and any native embedding draw above it.
        host.addView(HydrolysisGpuBand(this, session), 0)
        return host
    }
}
