package dev.waterui.android

import android.app.Activity
import android.content.res.Configuration
import android.os.Bundle
import android.widget.FrameLayout

/**
 * The host `Activity`: builds the root [FrameLayout], mounts the Rust app
 * into it, and forwards the lifecycle the backend needs — configuration
 * changes (theme, locale, density) and memory pressure. Rendering logic
 * lives in Rust; this class only carries the platform surface.
 */
open class WaterActivity : Activity() {
    private var handle: Long = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        WaterRuntime.loadLibrary(this)
        val root = FrameLayout(this)
        setContentView(root)
        handle = WaterRuntime.nativeCreate(this, root)
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
}
