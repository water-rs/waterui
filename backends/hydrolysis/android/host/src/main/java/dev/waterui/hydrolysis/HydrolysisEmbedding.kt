package dev.waterui.hydrolysis

import android.content.Context
import android.view.View
import androidx.activity.BackEventCompat
import androidx.activity.OnBackPressedCallback
import androidx.activity.OnBackPressedDispatcher
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStoreOwner

/** Retains the mounted session across configuration recreation. */
internal class HydrolysisSessionHolder : ViewModel() {

    private var session: HydrolysisSession? = null

    /** The session for this owner, created on first mount. */
    fun session(context: Context): HydrolysisSession =
        session ?: HydrolysisSession(context.applicationContext).also { session = it }

    override fun onCleared() {
        // The store can clear while the view stays attached — a custom
        // owner, a Compose navigation entry — and `destroy` then only
        // defers until the view detaches. Park the session first so it
        // stops pumping and taking input meanwhile, commit the pending
        // autofill save, then end it; a configuration recreation never
        // reaches here.
        session?.let {
            it.setVisible(false)
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
     * Mounts the app in [nativeLibraryName] as a View: prepares the process
     * environment, loads the library, retains the session in a ViewModel of
     * [viewModelStoreOwner] under [key], and wires lifecycle visibility, system
     * back and close requests while the returned view is attached.
     * One mount per key per owner; a second simultaneous mount under the same
     * key fails the session's single-binding check.
     */
    fun createView(
        context: Context,
        lifecycleOwner: LifecycleOwner,
        viewModelStoreOwner: ViewModelStoreOwner,
        onBackPressedDispatcher: OnBackPressedDispatcher,
        nativeLibraryName: String,
        onCloseRequested: () -> Unit,
        createContentView: (HydrolysisSession) -> View,
        key: String = nativeLibraryName,
        logLevel: String? = null,
    ): View {
        HydrolysisEnvironment.prepare(context)
        NativeBridge.load(nativeLibraryName, logLevel)
        val holder = ViewModelProvider(viewModelStoreOwner)[
            "dev.waterui.hydrolysis.session:$key",
            HydrolysisSessionHolder::class.java,
        ]
        val session = holder.session(context)
        val contentView = createContentView(session)
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
                    session.lifecycleOwner = lifecycleOwner
                    onBackPressedDispatcher.addCallback(lifecycleOwner, backCallback)
                    session.onBackAvailable = { available -> backCallback.isEnabled = available }
                    backCallback.isEnabled = session.backAvailable
                    session.onCloseRequested = onCloseRequested
                    lifecycleOwner.lifecycle.addObserver(observer)
                    // The attach listener fires after the content view's own
                    // `onAttachedToWindow`, so `bind` has already applied the
                    // lifecycle's current visibility state.
                }

                override fun onViewDetachedFromWindow(view: View) {
                    backCallback.remove()
                    lifecycleOwner.lifecycle.removeObserver(observer)
                    session.onBackAvailable = null
                    session.onCloseRequested = null
                    session.lifecycleOwner = null
                    // Visibility parks in `HydrolysisSession.unbind`, which
                    // the host view's own `onDetachedFromWindow` runs before
                    // this listener — a session destroyed in between must
                    // never see another call.
                }
            },
        )
        return contentView
    }
}
