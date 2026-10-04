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
        // The "webview" platform-view kind the app's tree mounts between the
        // controls: a real android.webkit.WebView with a pinned local
        // document (no network), so TalkBack, z-order and touch-ownership
        // checks run against a real native child.
        host.platformViewRegistry.registerFactory("webview") { context ->
            android.webkit.WebView(context).apply {
                settings.apply {
                    javaScriptEnabled = false
                    builtInZoomControls = false
                }
                loadData(
                    "<html><body><h1>Embedded WebView</h1>" +
                        "<p>A real platform view mounted inside the WaterUI " +
                        "tree: scrolling, links and its own accessibility " +
                        "tree all belong to it.</p>" +
                        "<a href=\"https://example.com\">A link</a></body></html>",
                    "text/html",
                    "utf-8",
                )
            }
        }
        return host
    }
}
