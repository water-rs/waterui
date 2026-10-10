package dev.waterui.hydrolysis.webview

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.MutableContextWrapper
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
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.ScriptHandler
import androidx.webkit.WebMessageCompat
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import dev.waterui.hydrolysis.CalledFromNative
import dev.waterui.hydrolysis.HostRebindAware
import dev.waterui.hydrolysis.HydrolysisSession
import java.io.ByteArrayInputStream
import java.util.concurrent.locks.ReentrantReadWriteLock
import kotlin.concurrent.read
import kotlin.concurrent.write

private val LOG_TAG = HydrolysisWebViewClient.LOG_TAG

/**
 * One asset reply crossing JNI: the status line's numeric code, the header
 * block as newline-joined `Name: value` lines, and the body.
 *
 * Constructed by `nativeAssetRespond` on the Rust side by name; the keep
 * annotation plus the `<init>` signature rule in
 * `webview/consumer-rules.pro` preserve the constructor through R8.
 */
class AssetResponse
@CalledFromNative
constructor(val status: Int, val headers: String, val body: ByteArray) {
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
 * configuration change — and [onHostRebound] retargets the wrapper and
 * re-observes the Lifecycle of the new Activity.
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
    private val assetHost: String,
) : WebView(context), HostRebindAware {
    /** The `Box<Weak<SharedState>>` Rust leaked at `create`; zero after `release`. */
    private var nativeHandle: Long = 0

    /**
     * The `Box<AssetServer>` `create` was handed; the teardown takes it
     * after `destroy` and whoever ran the teardown frees it. Guarded by
     * [assetServerLock]: `destroy()` does not join Chromium's IO threads,
     * so a `shouldInterceptRequest` can still be mid-dispatch — the read
     * lock covers only the check-and-acquire, the `Arc` clone carries the
     * dispatch itself outside it, and the write lock takes the pointer.
     */
    private val assetServerLock = ReentrantReadWriteLock()
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
                    if (nativeHandle != 0L) {
                        nativeProgressChanged(nativeHandle, newProgress)
                    }
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
        @CalledFromNative
        @JvmStatic
        fun create(
            session: HydrolysisSession,
            instanceId: Long,
            nativeHandle: Long,
            assetServer: Long,
            bridgeObject: String,
            asyncResultObject: String,
            assetHost: String,
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
                    assetHost,
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
        //
        // The navigation natives forward raw callbacks — URL, main-frame
        // flag, progress — and Rust's `NavigationTracker` alone decides which
        // `WebViewEvent`s they report.
        @JvmStatic private external fun nativeOpenNavigation(handle: Long, url: String)

        @JvmStatic
        private external fun nativeShouldOverrideUrlLoading(
            handle: Long,
            url: String,
            isMainFrame: Boolean,
            isRedirect: Boolean,
        ): Boolean

        @JvmStatic
        private external fun nativePageStarted(handle: Long, url: String, progress: Int)

        @JvmStatic private external fun nativeHistoryUpdated(handle: Long, url: String)

        @JvmStatic private external fun nativeProgressChanged(handle: Long, progress: Int)

        @JvmStatic
        private external fun nativeReceivedError(
            handle: Long,
            url: String,
            isMainFrame: Boolean,
            message: String,
        )

        @JvmStatic private external fun nativeRenderProcessGone(handle: Long, message: String)

        @JvmStatic
        private external fun nativeSslError(handle: Long, url: String?, message: String)

        @JvmStatic
        private external fun nativeNavigationState(
            handle: Long,
            canGoBack: Boolean,
            canGoForward: Boolean,
        )

        @JvmStatic
        private external fun nativeOnBridgeMessage(
            handle: Long,
            envelope: String,
            replyProxy: JavaScriptReplyProxy,
        )

        @JvmStatic
        private external fun nativeOriginMayUseBridge(handle: Long, origin: String): Boolean

        @JvmStatic private external fun nativeReleased(handle: Long)

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

        @JvmStatic private external fun nativeAssetServerAcquire(server: Long): Long

        @JvmStatic private external fun nativeAssetServerRelease(server: Long)

        @JvmStatic private external fun nativeFreeAssetServer(server: Long)
    }

    // ------------------------------------------------------------------
    // The surface Rust calls on the handle.
    // ------------------------------------------------------------------

    @CalledFromNative
    fun setUserAgent(userAgent: String) {
        settings.userAgentString = userAgent
    }

    /**
     * Replaces the document-start source list: the bridge sources first, then
     * the user scripts in first-injection order — the list `inject_script`
     * composes in Rust.
     */
    @CalledFromNative
    fun setDocumentStartScripts(sources: Array<String>) {
        documentStartSources = sources.toList()
        rebuildDocumentStartScripts()
    }

    /**
     * Replaces the admitted origins with the androidx rule strings Rust
     * computed, then reinstalls the bridge listener and rebuilds the
     * document-start scripts under the new rules.
     */
    @CalledFromNative
    fun setBridgeOrigins(rules: Array<String>) {
        originRules = rules.toList()
        installBridgeListener()
        rebuildDocumentStartScripts()
    }

    // ------------------------------------------------------------------
    // Application-started loads. Android reports none of them to
    // `shouldOverrideUrlLoading`, so each reports its target to Rust
    // itself — never a bare `loadUrl`/`goBack`/... from Rust.
    //
    // The target is always the engine's spelling of the URL, never the
    // application's: every later callback carries the engine's, and the
    // tracker matches the commit against it. `loadUrl` and `reload` report
    // after the call, because only then does the engine hold the entry it
    // is loading; that is in time, because every `WebViewClient` and
    // `WebChromeClient` callback is posted to this looper rather than
    // called from inside them.
    // ------------------------------------------------------------------

    /**
     * `loadUrl`, then the URL the engine is loading. On the UI thread
     * `loadUrl` creates the navigation's pending entry before it returns,
     * and `getUrl()` is the visible entry, which for a load the application
     * starts is that pending entry: the application's string after URL
     * fixup and canonicalization, so `https://waterui.dev#b` reads back as
     * `https://waterui.dev/#b`. A URL the engine refuses — invalid after
     * fixup, or too long — leaves no pending entry, and on a view with no
     * document `getUrl()` is then null. On a view that shows a document it
     * is that document's URL instead, the same answer a load of that very
     * URL gives (the engine turns it into a reload): this API cannot tell
     * the two apart, and the refused load is then opened and never ends.
     */
    @CalledFromNative
    fun navigateTo(url: String) {
        loadUrl(url)
        val target =
            this.url
                ?: throw IllegalStateException(
                    "hydrolysis webview: the system WebView refused to load \"$url\"",
                )
        openNavigation(target)
    }

    /**
     * `goBack`, when there is an entry to go back to. A history step never
     * makes its entry the visible one before it commits, so the target is
     * read from the back-forward list first; its URL is already the
     * engine's.
     */
    @CalledFromNative
    fun navigateBack() {
        val target = historyTarget(-1) ?: return
        openNavigation(target)
        goBack()
    }

    /** `goForward`, when there is an entry to go forward to; see [navigateBack]. */
    @CalledFromNative
    fun navigateForward() {
        val target = historyTarget(1) ?: return
        openNavigation(target)
        goForward()
    }

    /**
     * `reload`, then the URL of the entry it reloads: the current document,
     * which drops a load still pending, or the pending entry of the view's
     * first load. `getUrl()` is that entry's URL once `reload` returns; with
     * no document `reload` starts nothing and `getUrl()` is null.
     */
    @CalledFromNative
    fun navigateReload() {
        reload()
        val target = url ?: return
        openNavigation(target)
    }

    /** The URL of the history entry `offset` steps from the current one. */
    private fun historyTarget(offset: Int): String? {
        val history = copyBackForwardList()
        val index = history.currentIndex + offset
        if (index < 0 || index >= history.size) {
            return null
        }
        return history.getItemAtIndex(index).url
    }

    private fun openNavigation(url: String) {
        if (nativeHandle != 0L) {
            nativeOpenNavigation(nativeHandle, url)
        }
    }

    @CalledFromNative
    fun setCookie(url: String, cookie: String) {
        CookieManager.getInstance().setCookie(url, cookie)
    }

    /**
     * Replies through `nativeCookies` with the request header the cookie jar
     * would send for the current document — one `name=value` pair per line.
     * Before the first navigation the jar has no document, so the reply is
     * the empty header. The reply is synchronous, so the call is never in
     * `pendingCalls` — its settlement cannot be deferred.
     */
    @CalledFromNative
    fun getCookies(callId: Long) {
        val header =
            url
                ?.let { CookieManager.getInstance().getCookie(it) }
                ?.split(';')
                ?.map { it.trim() }
                ?.filter { it.isNotEmpty() }
                ?.joinToString("\n") ?: ""
        if (nativeHandle != 0L) {
            nativeCookies(nativeHandle, callId, header)
        }
    }

    /**
     * `evaluateJavascript` — the synchronous reply is already JSON, and the
     * async-call wrapper makes the awaiting case settle through
     * `nativeAsyncResult` instead.
     */
    @CalledFromNative
    fun evaluate(script: String, callId: Long) {
        evaluateJavascript(script) { value ->
            // A dead handle drops the reply in Rust; the call was already
            // settled by the release or `nativeDocumentReplaced` anyway.
            val handle = nativeHandle
            if (handle != 0L) {
                nativeJsResult(handle, callId, true, value ?: "null")
            }
        }
    }

    /**
     * Posts a bridge reply — the `Reply::message` the native side rendered for
     * the call it answered — through [replyProxy], the channel the listener
     * handed `nativeOnBridgeMessage` with that call. The engine binds the
     * proxy to the document that sent the call and drops the post once that
     * document is gone, so the reply never reaches another page; a released
     * view drops it here.
     */
    @CalledFromNative
    fun postBridgeReply(replyProxy: JavaScriptReplyProxy, message: String) {
        if (released) {
            return
        }
        replyProxy.postMessage(message)
    }

    /**
     * A new host view bound: the `MutableContextWrapper` retargets the new
     * context and the lifecycle belongs to the new Activity. A released
     * view skips both — the registration stays for the leaf's geometry,
     * but a destroyed WebView must not retarget its context wrapper or
     * re-observe a lifecycle it can no longer navigate with.
     */
    override fun onHostRebound(context: Context) {
        if (released) {
            return
        }
        (this.context as MutableContextWrapper).baseContext = context
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
        WebViewCompat.addWebMessageListener(
            this,
            asyncResultObject,
            setOf("*"),
        ) { _, message, _, isMainFrame, _ ->
            // The calls only ever run in the main frame, so a subframe's
            // post is dropped outright. Admission itself is the per-call
            // token the composed script stamped — checked in Rust, where an
            // opaque origin (`data:`, `about:blank`) or an IPv6 literal
            // cannot silently drop a real result the way a rebuilt
            // scheme://host[:port] comparison would.
            if (!isMainFrame) {
                Log.w(LOG_TAG, "an async result from a subframe was dropped")
                return@addWebMessageListener
            }
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
        ) { _, message, sourceOrigin, isMainFrame, replyProxy ->
            onBridgeMessage(message, sourceOrigin, isMainFrame, replyProxy)
        }
        bridgeListenerInstalled = true
    }

    private fun rebuildDocumentStartScripts() {
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
        replyProxy: JavaScriptReplyProxy,
    ) {
        if (!isMainFrame) {
            Log.w(LOG_TAG, "a bridge message from a subframe was dropped")
            return
        }
        val origin = sourceOrigin?.toString().orEmpty()
        if (nativeHandle == 0L || !nativeOriginMayUseBridge(nativeHandle, origin)) {
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
        nativeOnBridgeMessage(nativeHandle, envelope, replyProxy)
    }

    // ------------------------------------------------------------------
    // Reports the client callbacks share.
    // ------------------------------------------------------------------

    private fun emitSslError(url: String?, message: String) {
        if (nativeHandle != 0L) {
            nativeSslError(nativeHandle, url, message)
        }
    }

    private fun emitNavigationState() {
        if (nativeHandle != 0L) {
            nativeNavigationState(nativeHandle, canGoBack(), canGoForward())
        }
    }

    // ------------------------------------------------------------------
    // Teardown — the Kotlin runtime's `release()` order.
    // ------------------------------------------------------------------

    /**
     * Rust dropped its handle — in a frame that removed the leaf, or inside
     * `nativeDestroySession`, which drops the view tree that owns the
     * handle while the session itself is half dropped. Nothing here calls
     * back into Rust: the drop drains its call registry itself once this
     * returns, and frees the asset server this hands back.
     *
     * @return the `Box<AssetServer>` pointer, now the caller's to free, or
     *   zero when there is none or the render-process-gone teardown already
     *   freed it.
     */
    @CalledFromNative
    fun release(): Long {
        nativeHandle = 0L
        val server = tearDown()
        // Only the Rust-driven release unregisters: a render-process-gone
        // teardown keeps the instance so the still-published placement does
        // not crash `createSlot` — the leaf shows nothing until the Rust
        // handle drops.
        session.unregisterPlatformViewInstance(instanceId)
        return server
    }

    /**
     * The render process died: a `WebViewClient` callback the looper
     * delivers, so no Rust frame is under it and the native side is told
     * here — `nativeReleased` first, which drains the call registry
     * (including the async ids Kotlin forgot after the started sentinel)
     * and marks it dead: an event handler answering the `Error`
     * `nativeRenderProcessGone` then reports gets the dead-view value back
     * instead of dispatching `loadUrl`/`reload` onto a view `tearDown`
     * destroys next. The Rust handle that owns the view is dropped
     * afterwards all the same.
     */
    private fun releaseAfterRenderProcessGone(message: String) {
        val handle = nativeHandle
        nativeHandle = 0L
        if (handle != 0L) {
            nativeReleased(handle)
            nativeRenderProcessGone(handle, message)
        }
        val server = tearDown()
        if (server != 0L) {
            nativeFreeAssetServer(server)
        }
    }

    /**
     * Destroys the `WebView`, once, without calling into native code, and
     * returns the asset server pointer it took — zero on a second call.
     */
    private fun tearDown(): Long {
        if (released) {
            return 0L
        }
        released = true
        observedLifecycle?.removeObserver(lifecycleObserver)
        observedLifecycle = null
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
        // `destroy()` does not join Chromium's IO threads: an interception
        // can still be about to acquire the server, which is why the pointer
        // is taken under the write lock whose read half the acquire holds.
        // Once it is zero no interception can reach the `Box`, and one
        // already dispatching holds its own `Arc` clone.
        return assetServerLock.write {
            val server = assetServerPtr
            assetServerPtr = 0L
            server
        }
    }

    // ------------------------------------------------------------------
    // The client — the navigation/lifecycle half of the bridge.
    // ------------------------------------------------------------------

    private inner class HydrolysisClient : HydrolysisWebViewClient() {
        override fun shouldInterceptRequest(
            view: WebView,
            request: WebResourceRequest,
        ): WebResourceResponse? {
            val uri = request.url
            if (uri.scheme != "https" || uri.host != assetHost) {
                return null
            }
            // Runs on a WebView worker thread. The read lock covers only
            // the acquire: `nativeAssetRespond` runs outside it on an `Arc`
            // clone, so a slow request never stalls `release`'s write half.
            val server =
                assetServerLock.read {
                    if (assetServerPtr == 0L) {
                        0L
                    } else {
                        nativeAssetServerAcquire(assetServerPtr)
                    }
                }
            if (server == 0L) {
                return null
            }
            return try {
                nativeAssetRespond(
                        server,
                        request.method,
                        uri.encodedPath ?: "/",
                        uri.encodedQuery,
                    )
                    ?.toWebResourceResponse()
            } finally {
                nativeAssetServerRelease(server)
            }
        }

        override fun shouldOverrideUrlLoading(
            view: WebView,
            request: WebResourceRequest,
        ): Boolean {
            val handle = nativeHandle
            if (handle == 0L) {
                return false
            }
            // `true` cancels the request: a server redirect while redirects
            // are disabled. Cancelling it ends the load, which is the
            // blocked navigation's stop.
            return nativeShouldOverrideUrlLoading(
                handle,
                request.url.toString(),
                request.isForMainFrame,
                request.isRedirect,
            )
        }

        override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
            // The one callback that fires exactly on a main-frame
            // cross-document commit — no dedup: `nativePageStarted` drains
            // the calls the old document owed and bumps the generation, so
            // a stale result can never settle a new call.
            val handle = nativeHandle
            if (handle != 0L) {
                nativePageStarted(handle, url, view.progress)
            }
            emitNavigationState()
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String, isReload: Boolean) {
            val handle = nativeHandle
            if (handle != 0L) {
                nativeHistoryUpdated(handle, url)
            }
            emitNavigationState()
        }

        override fun onPageFinished(view: WebView, url: String) {
            // Not the navigation's finish: the system WebView also fires it
            // for a same-document history update mid-load. The finish is the
            // 100% progress report — see `NavigationTracker`.
            emitNavigationState()
        }

        override fun onReceivedError(
            view: WebView,
            request: WebResourceRequest,
            error: WebResourceError,
        ) {
            val handle = nativeHandle
            if (handle != 0L) {
                nativeReceivedError(
                    handle,
                    request.url.toString(),
                    request.isForMainFrame,
                    error.description?.toString() ?: "navigation failed",
                )
            }
            emitNavigationState()
        }

        override fun onReceivedSslError(
            view: WebView,
            handler: SslErrorHandler,
            error: SslError,
        ) {
            emitSslError(error.url, error.toString())
            handler.cancel()
        }

        override fun onRenderProcessGone(
            view: WebView,
            detail: RenderProcessGoneDetail,
        ): Boolean {
            releaseAfterRenderProcessGone(
                if (detail.didCrash()) {
                    "the web content process crashed"
                } else {
                    "the system reclaimed the web content process"
                },
            )
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
