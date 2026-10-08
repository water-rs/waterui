package dev.waterui.hydrolysis.webview

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.graphics.Bitmap
import android.net.Uri
import android.util.Log
import android.webkit.CookieManager
import android.webkit.RenderProcessGoneDetail
import android.webkit.SslErrorHandler
import android.net.http.SslError
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import dev.waterui.hydrolysis.MutableContextWrapper
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import androidx.webkit.ScriptHandler
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import dev.waterui.hydrolysis.HostRebindAware
import dev.waterui.hydrolysis.HydrolysisSession
import java.io.ByteArrayInputStream

/**
 * The asset origin `waterui-webview` gives a page that was opened with an
 * `AssetServer` — `ASSET_HTTPS_ORIGIN` on the Rust side. Requests to its host
 * are intercepted and answered through `nativeAssetRespond`.
 */
private const val ASSET_HTTPS_HOST = "waterui.localhost"

private val LOG_TAG = HydrolysisWebViewClient.LOG_TAG

/**
 * One asset reply crossing JNI: the status line's numeric code, the header
 * block as newline-joined `Name: value` lines, and the body.
 *
 * Constructed by `nativeAssetRespond` on the Rust side by name; the keep
 * rules in `consumer-rules.pro` preserve the constructor's signature.
 */
class AssetResponse(val status: Int, val headers: String, val body: ByteArray) {
    fun toWebResourceResponse(): WebResourceResponse {
        var mimeType: String? = null
        var encoding: String? = null
        val responseHeaders = mutableMapOf<String, String>()
        for (line in headers.split('\n')) {
            val separator = line.indexOf(':')
            if (separator <= 0) {
                continue
            }
            val name = line.substring(0, separator).trim()
            val value = line.substring(separator + 1).trim()
            if (name.equals("content-type", ignoreCase = true)) {
                mimeType = value.substringBefore(';').trim()
                val charset = value.substringAfter(';', "")
                if (charset.contains("charset=")) {
                    encoding = charset.substringAfter("charset=").trim()
                }
            } else {
                responseHeaders[name] = value
            }
        }
        return WebResourceResponse(
            mimeType ?: "application/octet-stream",
            encoding,
            status,
            assetReasonPhrase(status),
            responseHeaders,
            ByteArrayInputStream(body),
        )
    }
}

private fun assetReasonPhrase(status: Int): String =
    when (status) {
        200 -> "OK"
        204 -> "No Content"
        400 -> "Bad Request"
        403 -> "Forbidden"
        404 -> "Not Found"
        405 -> "Method Not Allowed"
        416 -> "Range Not Satisfiable"
        500 -> "Internal Server Error"
        else -> "Status $status"
    }

/**
 * The system `WebView` `waterui_webview` drives on Android: mounted through
 * the platform-view seam as a registered *instance* of its session, driven
 * from Rust over JNI.
 *
 * The JavaScript side — the transport, the async-result listener and the
 * document-start sources — is composed entirely on the Rust side; this class
 * applies the source lists and rule lists it is handed and performs no string
 * substitution.
 *
 * The view is created with a [MutableContextWrapper] over the session's
 * bound host context: a retained session rebinds a new host view — a
 * configuration change — and `HydrolysisSession.bind` retargets the wrapper
 * and calls [onHostRebound] so the Lifecycle observer follows the new
 * Activity.
 */
// JavaScript is the WebView's contract, and WEB_MESSAGE_LISTENER is a
// create-time invariant — the runtime carries these same suppressions on
// its own view.
@SuppressLint("ViewConstructor", "SetJavaScriptEnabled", "RequiresFeature")
class HydrolysisWebView
private constructor(
    private val session: HydrolysisSession,
    private val instanceId: Long,
    context: MutableContextWrapper,
    assetServer: Long,
    private val bridgeObjectName: String,
    asyncResultObjectName: String,
) : WebView(context), HostRebindAware {
    /** The `Box<Weak<SharedState>>` Rust leaked at `create`; zero after `release`. */
    private var nativeHandle: Long = 0

    /** The `Box<AssetServer>` `create` was handed; freed in `release` after `destroy`. */
    private var assetServerPtr: Long = assetServer

    /** Document-start sources in the order Rust pushed them, bridge sources first. */
    private var documentStartSources: List<String> = emptyList()

    /** Handles for the sources currently installed, dropped on every rebuild. */
    private val installedScripts = mutableListOf<ScriptHandler>()

    /**
     * The bridge origin policy, as the androidx rule strings Rust converted
     * `OriginPolicy::wire` tokens into. Empty denies every document, which is
     * a policy and not the absence of one.
     */
    private var originRules: List<String> = emptyList()
    private var bridgeListenerInstalled = false

    /** Call ids with a `nativeJsResult` reply still owed. */
    private val pendingCalls = HashSet<Long>()
    private var redirectsEnabled = true

    /**
     * The navigation already reported to Rust, cleared once it is over.
     *
     * Several client callbacks see one navigation, so this is what keeps them
     * from reporting it several times over.
     */
    private var announcedNavigationUrl: String? = null
    private var released = false

    /** The bound Activity's lifecycle, re-observed on every host rebind. */
    private var observedLifecycle: Lifecycle? = null
    private val lifecycleObserver =
        LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_PAUSE -> onPause()
                Lifecycle.Event.ON_RESUME -> onResume()
                else -> {}
            }
        }

    init {
        CookieManager.getInstance().setAcceptCookie(true)
        settings.javaScriptEnabled = true
        settings.domStorageEnabled = true
        settings.javaScriptCanOpenWindowsAutomatically = true
        installAsyncResultListener(asyncResultObjectName)
        webChromeClient =
            object : WebChromeClient() {
                override fun onProgressChanged(view: WebView, newProgress: Int) {
                    emitLoading(newProgress / 100f)
                }
            }
        webViewClient = HydrolysisClient()
    }

    companion object {
        /**
         * The JNI entry point `open` calls: builds the WebView on the
         * session's bound host context, wires the listeners, and registers it
         * as platform-view instance `instanceId`.
         */
        @JvmStatic
        fun create(
            session: HydrolysisSession,
            instanceId: Long,
            nativeHandle: Long,
            assetServer: Long,
            bridgeObject: String,
            asyncResultObject: String,
        ): HydrolysisWebView {
            requireWebMessageListener()
            val context =
                session.boundContext
                    ?: throw IllegalStateException(
                        "hydrolysis webview: cannot create a WebView on an unbound session",
                    )
            if (context.findActivity() == null) {
                throw IllegalStateException(
                    "hydrolysis webview: the session's bound context is not an Activity context",
                )
            }
            val view =
                HydrolysisWebView(
                    session,
                    instanceId,
                    MutableContextWrapper(context),
                    assetServer,
                    bridgeObject,
                    asyncResultObject,
                )
            view.nativeHandle = nativeHandle
            view.observeLifecycle(context)
            session.registerPlatformViewInstance(instanceId, view)
            return view
        }

        private fun requireWebMessageListener() {
            if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
                throw IllegalStateException(
                    "hydrolysis webview: the system WebView lacks WebViewFeature." +
                        "WEB_MESSAGE_LISTENER — the bridge cannot run without it",
                )
            }
            if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
                throw IllegalStateException(
                    "hydrolysis webview: the system WebView lacks WebViewFeature." +
                        "DOCUMENT_START_SCRIPT — the bridge cannot run without it",
                )
            }
        }

        // The natives `catch_unwind` guards on the Rust side; a released
        // handle upgrades to nothing and the callback is dropped there.
        @JvmStatic private external fun nativeWillNavigate(handle: Long, url: String)

        @JvmStatic private external fun nativeLoading(handle: Long, progress: Float)

        @JvmStatic private external fun nativeLoaded(handle: Long)

        @JvmStatic
        private external fun nativeRedirect(handle: Long, from: String, to: String)

        @JvmStatic private external fun nativeError(handle: Long, message: String)

        @JvmStatic
        private external fun nativeSslError(handle: Long, url: String, message: String)

        @JvmStatic
        private external fun nativeNavigationState(
            handle: Long,
            canGoBack: Boolean,
            canGoForward: Boolean,
        )

        @JvmStatic
        private external fun nativeOnBridgeMessage(handle: Long, envelope: String)

        @JvmStatic private external fun nativeAsyncResult(handle: Long, payload: String)

        @JvmStatic
        private external fun nativeJsResult(
            handle: Long,
            callId: Long,
            ok: Boolean,
            value: String,
        )

        @JvmStatic
        private external fun nativeCookies(handle: Long, callId: Long, cookiesHeader: String)

        /**
         * Runs on a WebView worker thread. Answers with the `AssetResponse`
         * `nativeAssetRespond` builds, or null — unreachable in practice,
         * interception only fires while a server pointer is installed.
         */
        @JvmStatic
        private external fun nativeAssetRespond(
            server: Long,
            method: String,
            path: String,
            query: String?,
        ): AssetResponse?

        @JvmStatic private external fun nativeFreeAssetServer(server: Long)
    }

    // ------------------------------------------------------------------
    // The surface Rust calls on the handle.
    // ------------------------------------------------------------------

    fun setUserAgent(userAgent: String) {
        settings.userAgentString = userAgent
    }

    /**
     * Replaces the document-start source list: the bridge sources first, then
     * the user scripts in first-injection order — the list `inject_script`
     * composes in Rust.
     */
    fun setDocumentStartScripts(sources: Array<String>) {
        documentStartSources = sources.toList()
        rebuildDocumentStartScripts()
    }

    /**
     * Replaces the admitted origins with the androidx rule strings Rust
     * computed, then reinstalls the bridge listener and rebuilds the
     * document-start scripts under the new rules.
     */
    fun setBridgeOrigins(rules: Array<String>) {
        originRules = rules.toList()
        installBridgeListener()
        rebuildDocumentStartScripts()
    }

    fun setRedirectsEnabled(enabled: Boolean) {
        redirectsEnabled = enabled
    }

    fun setCookie(url: String, cookie: String) {
        CookieManager.getInstance().setCookie(url, cookie)
    }

    /**
     * Replies through `nativeCookies` with the request header the cookie jar
     * would send for the current document — one `name: value` pair per line.
     * Before the first navigation the jar has no document, so the reply is
     * the empty header.
     */
    fun getCookies(callId: Long) {
        pendingCalls += callId
        val header =
            url
                ?.let { CookieManager.getInstance().getCookie(it) }
                ?.split(';')
                ?.map { it.trim() }
                ?.filter { it.isNotEmpty() }
                ?.joinToString("\n") ?: ""
        pendingCalls -= callId
        nativeCookies(nativeHandle, callId, header)
    }

    /**
     * `evaluateJavascript` — the synchronous reply is already JSON, and the
     * async-call wrapper makes the awaiting case settle through
     * `nativeAsyncResult` instead.
     */
    fun evaluate(script: String, callId: Long) {
        pendingCalls += callId
        evaluateJavascript(script) { value ->
            pendingCalls -= callId
            nativeJsResult(nativeHandle, callId, true, value ?: "null")
        }
    }

    /**
     * Evaluates a bridge reply: the `resolve_script` `nativeOnBridgeMessage`
     * composed for the call it answered.
     */
    fun evaluateBridgeScript(script: String) {
        evaluateJavascript(script, null)
    }

    /** A new host view bound: the lifecycle belongs to the new Activity. */
    override fun onHostRebound(context: Context) {
        observeLifecycle(context)
    }

    private fun observeLifecycle(context: Context) {
        observedLifecycle?.removeObserver(lifecycleObserver)
        observedLifecycle = (context.findActivity() as? LifecycleOwner)?.lifecycle
        observedLifecycle?.addObserver(lifecycleObserver)
    }

    // ------------------------------------------------------------------
    // Bridge and script installation.
    // ------------------------------------------------------------------

    private fun installAsyncResultListener(asyncResultObject: String) {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
            return
        }
        WebViewCompat.addWebMessageListener(
            this,
            asyncResultObject,
            setOf("*"),
        ) { _, message, _, _, _ ->
            val payload = message.data
            if (payload != null) {
                nativeAsyncResult(nativeHandle, payload)
            }
        }
    }

    private fun installBridgeListener() {
        if (bridgeListenerInstalled) {
            WebViewCompat.removeWebMessageListener(this, bridgeObjectName)
            bridgeListenerInstalled = false
        }
        if (originRules.isEmpty()) {
            return
        }
        WebViewCompat.addWebMessageListener(
            this,
            bridgeObjectName,
            originRules.toSet(),
        ) { _, message, sourceOrigin, isMainFrame, _ ->
            onBridgeMessage(message, sourceOrigin, isMainFrame)
        }
        bridgeListenerInstalled = true
    }

    private fun rebuildDocumentStartScripts() {
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
            return
        }
        installedScripts.forEach(ScriptHandler::remove)
        installedScripts.clear()
        if (originRules.isEmpty()) {
            return
        }
        for (source in documentStartSources) {
            installedScripts +=
                WebViewCompat.addDocumentStartJavaScript(this, source, originRules.toSet())
        }
    }

    private fun onBridgeMessage(
        message: WebMessageCompat,
        sourceOrigin: Uri?,
        isMainFrame: Boolean,
    ) {
        if (!isMainFrame) {
            Log.w(LOG_TAG, "a bridge message from a subframe was dropped")
            return
        }
        val origin = sourceOrigin?.toString().orEmpty()
        if (!originMayUseBridge(origin)) {
            Log.w(LOG_TAG, "a bridge message from unadmitted origin \"$origin\" was dropped")
            return
        }
        val envelope = message.data
        if (envelope == null) {
            Log.w(LOG_TAG, "a bridge message with a non-string payload was dropped")
            return
        }
        if (nativeHandle == 0L) {
            Log.w(LOG_TAG, "a bridge message on a released web view was dropped")
            return
        }
        nativeOnBridgeMessage(nativeHandle, envelope)
    }

    /**
     * Whether `origin` — the source `addWebMessageListener` reports — may use
     * the bridge. The rules here are already the androidx form: the runtime's
     * `file:` token arrives converted to `file://`.
     */
    private fun originMayUseBridge(origin: String): Boolean =
        originRules.any { rule ->
            when (rule) {
                "*" -> true
                "file://" -> origin.startsWith("file://")
                else -> rule == origin
            }
        }

    // ------------------------------------------------------------------
    // Events — one native callback per Rust `WebViewEvent`/`BackendEvent`.
    // ------------------------------------------------------------------

    private fun announceNavigation(url: String) {
        if (url.isEmpty()) {
            return
        }
        if (url == announcedNavigationUrl) {
            return
        }
        announcedNavigationUrl = url
        if (nativeHandle != 0L) {
            nativeWillNavigate(nativeHandle, url)
        }
    }

    private fun finishNavigationAnnouncement() {
        announcedNavigationUrl = null
    }

    private fun emitLoading(progress: Float) {
        if (nativeHandle != 0L) {
            nativeLoading(nativeHandle, progress)
        }
    }

    private fun emitLoaded() {
        if (nativeHandle != 0L) {
            nativeLoaded(nativeHandle)
        }
    }

    private fun emitRedirect(from: String, to: String) {
        if (nativeHandle != 0L) {
            nativeRedirect(nativeHandle, from, to)
        }
    }

    private fun emitError(message: String) {
        if (nativeHandle != 0L) {
            nativeError(nativeHandle, message)
        }
    }

    private fun emitSslError(url: String, message: String) {
        if (nativeHandle != 0L) {
            nativeSslError(nativeHandle, url, message)
        }
    }

    private fun emitNavigationState() {
        if (nativeHandle != 0L) {
            nativeNavigationState(nativeHandle, canGoBack(), canGoForward())
        }
    }

    private fun abandonPendingCalls(reason: String) {
        val abandoned = pendingCalls.toList()
        pendingCalls.clear()
        for (callId in abandoned) {
            nativeJsResult(nativeHandle, callId, false, reason)
        }
    }

    // ------------------------------------------------------------------
    // Teardown — the Kotlin runtime's `release()` order.
    // ------------------------------------------------------------------

    fun release() {
        // A dead render process tears this view down where it is reported,
        // and the Rust handle that owns the view is dropped afterwards all
        // the same.
        if (released) {
            return
        }
        released = true
        observedLifecycle?.removeObserver(lifecycleObserver)
        observedLifecycle = null
        // Every native callback still owed a result has to be settled before
        // the web view goes away: the Rust side is waiting on a channel whose
        // sender it handed us, and a callback that is dropped rather than
        // called leaves that sender leaked and its task suspended forever.
        abandonPendingCalls("the web view was closed")
        nativeHandle = 0L
        originRules = emptyList()
        documentStartSources = emptyList()
        // Nothing is admitted and nothing is left to inject, so this removes
        // the bridge listener and every script currently installed.
        installBridgeListener()
        rebuildDocumentStartScripts()
        stopLoading()
        webChromeClient = null
        webViewClient = HydrolysisWebViewClient()
        (parent as? android.view.ViewGroup)?.removeView(this)
        destroy()
        // `destroy` has stopped interception, so no `nativeAssetRespond` call
        // can still be in flight on a WebView worker thread.
        val server = assetServerPtr
        assetServerPtr = 0L
        if (server != 0L) {
            nativeFreeAssetServer(server)
        }
        session.unregisterPlatformViewInstance(instanceId)
    }

    // ------------------------------------------------------------------
    // The client — the navigation/lifecycle half of the bridge.
    // ------------------------------------------------------------------

    private inner class HydrolysisClient : HydrolysisWebViewClient() {
        override fun shouldInterceptRequest(
            view: WebView,
            request: WebResourceRequest,
        ): WebResourceResponse? {
            val server = assetServerPtr
            val uri = request.url
            if (server == 0L || uri.scheme != "https" || uri.host != ASSET_HTTPS_HOST) {
                return null
            }
            // Runs on a WebView worker thread — `nativeAssetRespond` is the
            // direct JNI entry point that exists precisely for that.
            return nativeAssetRespond(
                    server,
                    request.method,
                    uri.encodedPath ?: "/",
                    uri.encodedQuery,
                )
                ?.toWebResourceResponse()
        }

        override fun shouldOverrideUrlLoading(
            view: WebView,
            request: WebResourceRequest,
        ): Boolean {
            if (!request.isForMainFrame) {
                return false
            }
            val targetUrl = request.url?.toString() ?: return false
            if (!redirectsEnabled && request.isRedirect) {
                emitRedirect(announcedNavigationUrl ?: targetUrl, targetUrl)
                return true
            }
            announceNavigation(targetUrl)
            return false
        }

        override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
            abandonPendingCalls("the document was replaced before the script ran")
            announceNavigation(url)
            emitLoading(0f)
            emitNavigationState()
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) {
            announceNavigation(url)
            finishNavigationAnnouncement()
            emitNavigationState()
        }

        override fun onPageFinished(view: WebView, url: String) {
            finishNavigationAnnouncement()
            emitLoaded()
            emitNavigationState()
        }

        override fun onReceivedError(
            view: WebView,
            request: WebResourceRequest,
            error: WebResourceError,
        ) {
            if (!request.isForMainFrame) {
                return
            }
            finishNavigationAnnouncement()
            emitError(error.description?.toString() ?: "navigation failed")
            emitNavigationState()
        }

        override fun onReceivedSslError(
            view: WebView,
            handler: SslErrorHandler,
            error: SslError,
        ) {
            emitSslError(error.url ?: "", error.toString())
            handler.cancel()
        }

        override fun onRenderProcessGone(
            view: WebView,
            detail: RenderProcessGoneDetail,
        ): Boolean {
            finishNavigationAnnouncement()
            emitError(
                if (detail.didCrash()) {
                    "the web content process crashed"
                } else {
                    "the system reclaimed the web content process"
                },
            )
            release()
            return true
        }
    }
}

private tailrec fun Context.findActivity(): Activity? =
    when (this) {
        is Activity -> this
        is ContextWrapper -> baseContext.findActivity()
        else -> null
    }
