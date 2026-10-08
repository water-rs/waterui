package dev.waterui.hydrolysis.webview;

import android.util.Log;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.WebView;
import android.webkit.WebViewClient;

/**
 * The base for every client Hydrolysis attaches to a web view.
 *
 * <p>A render process can be killed under the application — it crashed, or the system reclaimed
 * its memory while the app was in the background — and the platform reports that here. A client
 * that does not answer the report makes the framework kill the application process along with the
 * renderer, so every client Hydrolysis installs answers it, and the only honest answer is that
 * this web view is finished: it can never draw or run a script again.
 *
 * <p>This is the only place that extends {@link WebViewClient}, and it is the only Java file in
 * the module, because the check that enforces this callback — {@code
 * MissingOnRenderProcessGone}, shipped as lint by {@code androidx.webkit} — reports every
 * construction of a {@code WebViewClient} unconditionally. Kotlin writes a superclass as a
 * constructor call, so a Kotlin subclass is reported whether or not it implements the callback,
 * while Java's {@code extends} is not a call and the check reads the implementation below.
 */
class HydrolysisWebViewClient extends WebViewClient {
    /** The tag every web view log line carries, in both languages. */
    static final String LOG_TAG = "WaterUI.WebView";

    @Override
    public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
        Log.w(LOG_TAG, "web content process gone (crashed=" + detail.didCrash() + ")");
        ViewParent parent = view.getParent();
        if (parent instanceof ViewGroup) {
            ((ViewGroup) parent).removeView(view);
        }
        view.destroy();
        return true;
    }
}
