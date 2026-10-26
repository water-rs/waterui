package dev.waterui.hydrolysis.testapp

import android.view.View
import dev.waterui.hydrolysis.HydrolysisSession
import dev.waterui.hydrolysis.SessionPainter
import dev.waterui.hydrolysis.hwui.HwuiPainter

/** The HWUI test app: the retained tree draws through the host's RenderNodes. */
class MainActivity : TestActivity() {
    override fun createPainter(): SessionPainter = HwuiPainter()

    override fun createContentView(session: HydrolysisSession): View = createHost(session)
}
