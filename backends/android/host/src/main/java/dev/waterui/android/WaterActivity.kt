package dev.waterui.android

import android.app.Activity
import android.content.res.Configuration
import android.os.Bundle
import android.view.View
import android.widget.FrameLayout
import androidx.core.view.OnApplyWindowInsetsListener
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsAnimationCompat
import androidx.core.view.WindowInsetsCompat

/**
 * The host `Activity`: builds the root [FrameLayout], mounts the Rust app
 * into it, and forwards the lifecycle the backend needs — configuration
 * changes (theme, locale, density) and memory pressure. Rendering logic
 * lives in Rust; this class only carries the platform surface.
 *
 * The window lays out edge-to-edge and forwards both safe-area regions —
 * the container region (system bars, cutouts, the caption bar) and the
 * keyboard region (the IME) — to Rust on every dispatch and every
 * animation frame. The IME never resizes the window; apps declare
 * `android:windowSoftInputMode="adjustNothing"` and avoid the keyboard
 * through the insets alone.
 */
open class WaterActivity : Activity() {
    private var handle: Long = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        WaterRuntime.loadLibrary(this)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        val root = FrameLayout(this)
        setContentView(root)
        handle = WaterRuntime.nativeCreate(this, root)
        val forwarder = InsetsForwarder(handle)
        ViewCompat.setOnApplyWindowInsetsListener(root, forwarder)
        ViewCompat.setWindowInsetsAnimationCallback(root, forwarder)
        ViewCompat.requestApplyInsets(root)
    }

    override fun onDestroy() {
        if (handle != 0L) {
            WaterRuntime.nativeDestroy(handle)
            handle = 0
        }
        super.onDestroy()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (handle != 0L) {
            WaterRuntime.nativeOnConfigurationChanged(handle)
        }
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (handle != 0L) {
            WaterRuntime.nativeOnTrimMemory(handle, level)
        }
    }

    /**
     * Pushes both safe-area regions to the runtime whenever the window's
     * insets change — on dispatch and on every `WindowInsetsAnimationCompat`
     * frame, so a keyboard move reaches the layout with the platform's own
     * animation. The insets pass through unconsumed: children of the root
     * keep their own dispatch.
     */
    private class InsetsForwarder(private val handle: Long) :
        WindowInsetsAnimationCompat.Callback(DISPATCH_MODE_CONTINUE_ON_SUBTREE),
        OnApplyWindowInsetsListener {

        override fun onApplyWindowInsets(v: View, insets: WindowInsetsCompat): WindowInsetsCompat {
            push(insets)
            return insets
        }

        override fun onProgress(
            insets: WindowInsetsCompat,
            runningAnimations: List<WindowInsetsAnimationCompat>,
        ): WindowInsetsCompat {
            push(insets)
            return insets
        }

        private fun push(insets: WindowInsetsCompat) {
            val container = insets.getInsets(
                WindowInsetsCompat.Type.systemBars()
                    or WindowInsetsCompat.Type.displayCutout()
                    or WindowInsetsCompat.Type.captionBar(),
            )
            val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime())
            WaterRuntime.nativeInsetsChanged(
                handle,
                container.left,
                container.top,
                container.right,
                container.bottom,
                keyboard.left,
                keyboard.top,
                keyboard.right,
                keyboard.bottom,
            )
        }
    }
}
