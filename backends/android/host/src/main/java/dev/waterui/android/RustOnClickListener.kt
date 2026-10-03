package dev.waterui.android

import android.view.View

/**
 * A `View.OnClickListener` whose click lands in Rust. The [handle] is the
 * backend's click-handler pointer, set at construction by Rust; nothing else
 * the listener does is Kotlin's business.
 */
class RustOnClickListener(private val handle: Long) : View.OnClickListener {
    override fun onClick(view: View) {
        nativeOnClick(handle)
    }

    private external fun nativeOnClick(handle: Long)
}
