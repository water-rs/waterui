package dev.waterui.android.reference

import android.annotation.SuppressLint
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView

private const val DEFAULT_URL = "file:///android_asset/bench_doc.html"

private class PageApi(private val onGreeting: (String) -> Unit) {
    @JavascriptInterface
    fun greet(name: String): String {
        val message = "Hello, $name!"
        onGreeting(message)
        return message
    }
}

/**
 * Compose twin of the webview fixture: a real android.webkit.WebView loading
 * the same pinned local document the Hydrolysis registry hosts, with the
 * address bar + Go, Back/Forward/Reload/Stop controls, the Allow-redirects
 * toggle, the Get Title (JS) bridge, status text and progress bar.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun WebviewTwin() {
    var address by remember { mutableStateOf(DEFAULT_URL) }
    var status by remember { mutableStateOf("Idle") }
    var progress by remember { mutableFloatStateOf(0f) }
    var allowRedirects by remember { mutableStateOf(true) }
    var jsResult by remember { mutableStateOf("(none)") }
    var greetings by remember { mutableIntStateOf(0) }
    var webView: WebView? by remember { mutableStateOf(null) }

    Column(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxWidth().padding(WATERUI_PADDING.dp),
            verticalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            TitleText("WebView Playground")

            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it },
                    modifier = Modifier.weight(1f).testTag("web:address"),
                    singleLine = true,
                )
                Button(
                    onClick = {
                        BenchMarkers.tap()
                        status = "Navigating to $address"
                        progress = 0f
                        webView?.loadUrl(address)
                    },
                    modifier = Modifier.testTag("web:go"),
                ) { Text("Go") }
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = { BenchMarkers.tap(); webView?.goBack() },
                    modifier = Modifier.testTag("web:back"),
                ) { Text("Back") }
                Button(
                    onClick = { BenchMarkers.tap(); webView?.goForward() },
                    modifier = Modifier.testTag("web:forward"),
                ) { Text("Forward") }
                Button(
                    onClick = { BenchMarkers.tap(); webView?.reload() },
                    modifier = Modifier.testTag("web:reload"),
                ) { Text("Reload") }
                Button(
                    onClick = { BenchMarkers.tap(); webView?.stopLoading() },
                    modifier = Modifier.testTag("web:stop"),
                ) { Text("Stop") }
            }

            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Allow redirects")
                Switch(
                    checked = allowRedirects,
                    onCheckedChange = {
                        BenchMarkers.tap()
                        allowRedirects = it
                    },
                    modifier = Modifier.testTag("web:redirects"),
                )
            }

            Button(
                onClick = {
                    BenchMarkers.tap()
                    webView?.evaluateJavascript("document.title") { result ->
                        jsResult = result ?: "JS error: no result"
                    }
                },
                modifier = Modifier.testTag("web:title"),
            ) { Text("Get Title (JS)") }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CaptionText("Status:")
                BodyText(status)
            }

            Column {
                CaptionText("Load progress")
                LinearProgressIndicator(
                    progress = { progress },
                    modifier = Modifier.fillMaxWidth().testTag("web:progress"),
                )
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CaptionText("JS Result:")
                BodyText(jsResult)
            }
            FootnoteText("PageApi greetings: $greetings")
        }

        AndroidView(
            factory = { ctx ->
                WebView(ctx).apply {
                    settings.javaScriptEnabled = true
                    addJavascriptInterface(
                        PageApi { msg ->
                            greetings += 1
                            jsResult = msg
                        },
                        "PageApi",
                    )
                    webViewClient = object : WebViewClient() {
                        override fun shouldOverrideUrlLoading(
                            view: WebView,
                            request: android.webkit.WebResourceRequest,
                        ): Boolean {
                            val to = request.url.toString()
                            return if (allowRedirects || !request.isRedirect) {
                                address = to
                                status = "Navigating to $to"
                                false
                            } else {
                                status = "Redirect blocked: $to"
                                true
                            }
                        }

                        override fun onPageFinished(view: WebView, url: String) {
                            status = "Loaded"
                            progress = 1f
                        }

                        override fun onReceivedError(
                            view: WebView,
                            request: android.webkit.WebResourceRequest,
                            error: android.webkit.WebResourceError,
                        ) {
                            if (request.isForMainFrame) {
                                status = "Error: ${error.description}"
                            }
                        }
                    }
                    webChromeClient = object : WebChromeClient() {
                        override fun onProgressChanged(view: WebView, newProgress: Int) {
                            progress = newProgress / 100f
                            if (newProgress in 1..99) {
                                status = "Loading $newProgress%"
                            }
                        }
                    }
                    loadUrl(DEFAULT_URL)
                }
            },
            update = { webView = it },
            modifier = Modifier.fillMaxSize().testTag("web:content"),
        )
    }
}
