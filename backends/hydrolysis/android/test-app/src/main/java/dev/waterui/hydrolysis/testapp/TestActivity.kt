package dev.waterui.hydrolysis.testapp

import dev.waterui.hydrolysis.HydrolysisActivity
import dev.waterui.hydrolysis.HydrolysisHostView
import dev.waterui.hydrolysis.HydrolysisSession

/**
 * The debug app exercising the Hydrolysis host end to end: the WaterUI tree
 * from `libhydrolysis_test_app.so` mounts on a [HydrolysisHostView] with the
 * selected painter (GPU band or HWUI session painter).
 */
abstract class TestActivity : HydrolysisActivity() {

    override val nativeLibraryName: String = "hydrolysis_test_app"

    /** The host view with the platform-view registry wired — painters add their surface. */
    protected fun createHost(session: HydrolysisSession): HydrolysisHostView {
        val host = HydrolysisHostView(this, session)
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
