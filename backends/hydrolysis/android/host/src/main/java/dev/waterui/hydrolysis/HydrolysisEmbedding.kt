package dev.waterui.hydrolysis

import android.content.Context
import android.view.View
import android.widget.FrameLayout
import androidx.activity.BackEventCompat
import androidx.activity.OnBackPressedCallback
import androidx.activity.OnBackPressedDispatcher
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.setViewTreeLifecycleOwner

/**
 * Retains the mounted session across configuration recreation, and routes
 * its close requests to the host currently mounting it.
 */
internal class HydrolysisSessionHolder : ViewModel() {

    private var session: HydrolysisSession? = null

    /**
     * The attached mount's close handler. It is cleared when the mount's view
     * detaches, so the holder never keeps a destroyed host — an activity torn
     * down by a configuration change — alive.
     */
    private var onCloseRequested: (() -> Unit)? = null

    /** A close arrived while no mount was attached; the next mount gets it. */
    private var closePending = false

    /** The session for this owner, created on first mount. */
    fun session(context: Context): HydrolysisSession =
        session ?: HydrolysisSession(context.applicationContext, ::requestClose)
            .also { session = it }

    /** A mount's view attached: it handles close requests from now on. */
    fun attach(onCloseRequested: () -> Unit) {
        this.onCloseRequested = onCloseRequested
        if (closePending) {
            closePending = false
            onCloseRequested()
        }
    }

    /** The mount's view detached: closes wait for the next mount. */
    fun detach() {
        onCloseRequested = null
    }

    private fun requestClose() {
        val handler = onCloseRequested
        if (handler == null) {
            closePending = true
        } else {
            handler()
        }
    }

    override fun onCleared() {
        // The store can clear while the view stays attached — a custom
        // owner, a Compose navigation entry — and `destroy` then only
        // defers until the view detaches. Commit the pending autofill save,
        // then destroy parks the session permanently. Window events and
        // input still reach the live native state until detach; a
        // configuration recreation never reaches here.
        session?.let {
            it.hostView?.autofillCommit()
            it.destroy()
        }
        session = null
    }
}

/**
 * The entry point that mounts a Hydrolysis app inside an existing host —
 * an activity, a fragment, or any other owner set — as an ordinary `View`.
 */
object HydrolysisEmbedding {

    /**
     * Mounts the app in [nativeLibraryName] as a View. The returned container
     * is what the host shows immediately — an empty view while
     * [HydrolysisEnvironment]'s asset sync runs off the main thread; the
     * mount continues on the main thread once it finishes: environment
     * overrides land through [applyEnvironmentOverrides], the library loads,
     * the session is retained in a ViewModel of [viewModelStoreOwner] under
     * [key], and its content view joins the container. [onMounted] runs last,
     * after the content view is wired. An owner whose lifecycle reaches
     * DESTROYED before the sync finishes never starts a session — the
     * container stays empty.
     *
     * [onCloseRequested] receives the app's close requests while the returned
     * view is attached. A close that arrives between mounts — after a
     * configuration change detached the old view and before the new one
     * attaches — waits and reaches the next mount's handler when its view
     * attaches.
     *
     * One mount per key per owner; a second simultaneous mount under the same
     * key fails the session's single-binding check.
     */
    fun createView(
        context: Context,
        lifecycleOwner: LifecycleOwner,
        viewModelStoreOwner: ViewModelStoreOwner,
        onBackPressedDispatcher: OnBackPressedDispatcher,
        nativeLibraryName: String,
        key: String = nativeLibraryName,
        logLevel: String? = null,
        applyEnvironmentOverrides: () -> Unit = {},
        createContentView: (HydrolysisSession) -> View,
        onCloseRequested: () -> Unit,
        onMounted: () -> Unit = {},
    ): View {
        val container = FrameLayout(context)
        HydrolysisEnvironment.prepare(context) {
            if (lifecycleOwner.lifecycle.currentState != Lifecycle.State.DESTROYED) {
                applyEnvironmentOverrides()
                NativeBridge.load(nativeLibraryName, logLevel)
                mountSession(
                    container, context, lifecycleOwner, viewModelStoreOwner,
                    onBackPressedDispatcher, key, createContentView,
                    onCloseRequested,
                )
                onMounted()
            }
        }
        return container
    }

    /**
     * The main-thread half of [createView], run once the asset sync finished:
     * the session is retained, its content view created and wired — lifecycle
     * visibility, system back, the owning mount's close handler — and added
     * to the container the caller already shows.
     */
    private fun mountSession(
        container: FrameLayout,
        context: Context,
        lifecycleOwner: LifecycleOwner,
        viewModelStoreOwner: ViewModelStoreOwner,
        onBackPressedDispatcher: OnBackPressedDispatcher,
        key: String,
        createContentView: (HydrolysisSession) -> View,
        onCloseRequested: () -> Unit,
    ) {
        val holder = ViewModelProvider(viewModelStoreOwner)[
            "dev.waterui.hydrolysis.session:$key",
            HydrolysisSessionHolder::class.java,
        ]
        val session = holder.session(context)
        val contentView = createContentView(session)
        contentView.setViewTreeLifecycleOwner(lifecycleOwner)
        val observer = object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                session.setVisible(true)
            }

            override fun onStop(owner: LifecycleOwner) {
                session.setVisible(false)
            }
        }
        val backCallback = object : OnBackPressedCallback(false) {
            override fun handleOnBackStarted(backEvent: BackEventCompat) {
                session.dispatchBack(
                    NativeBridge.BACK_STARTED,
                    backEvent.swipeEdge,
                    backEvent.progress.toDouble(),
                )
            }

            override fun handleOnBackProgressed(backEvent: BackEventCompat) {
                session.dispatchBack(
                    NativeBridge.BACK_PROGRESSED,
                    backEvent.swipeEdge,
                    backEvent.progress.toDouble(),
                )
            }

            override fun handleOnBackCancelled() {
                session.dispatchBack(NativeBridge.BACK_CANCELLED, 0, 0.0)
            }

            override fun handleOnBackPressed() {
                session.dispatchBack(NativeBridge.BACK_INVOKED, 0, 0.0)
            }
        }
        contentView.addOnAttachStateChangeListener(
            object : View.OnAttachStateChangeListener {
                override fun onViewAttachedToWindow(view: View) {
                    onBackPressedDispatcher.addCallback(lifecycleOwner, backCallback)
                    session.onBackAvailable = { available -> backCallback.isEnabled = available }
                    backCallback.isEnabled = session.backAvailable
                    lifecycleOwner.lifecycle.addObserver(observer)
                    // The attach listener fires after the content view's own
                    // `onAttachedToWindow`, so `bind` has already applied the
                    // lifecycle's current visibility state.
                    holder.attach(onCloseRequested)
                }

                override fun onViewDetachedFromWindow(view: View) {
                    holder.detach()
                    backCallback.remove()
                    lifecycleOwner.lifecycle.removeObserver(observer)
                    session.onBackAvailable = null
                    // Visibility parks in `HydrolysisSession.unbind`, which
                    // the host view's own `onDetachedFromWindow` runs before
                    // this listener — a session destroyed in between must
                    // never see another call.
                }
            },
        )
        container.addView(contentView)
    }
}
