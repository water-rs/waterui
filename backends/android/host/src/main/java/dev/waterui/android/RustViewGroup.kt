package dev.waterui.android

import android.content.Context
import android.widget.FrameLayout

/**
 * A `ViewGroup` whose measure and layout are driven by Rust. The backend
 * sets [handle] right after construction; every `onMeasure`/`onLayout`
 * forwards into the container leaf the handle names, which reports its size
 * and places its children itself.
 */
class RustViewGroup(context: Context) : FrameLayout(context) {
    private var handle: Long = 0

    /** The Rust state pointer the callbacks forward. Called once, by Rust. */
    fun setHandle(handle: Long) {
        this.handle = handle
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val packed = nativeMeasure(handle, widthMeasureSpec, heightMeasureSpec)
        setMeasuredDimension(
            (packed shr 32).toInt(),
            packed.toInt(),
        )
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        nativeLayout(handle, left, top, right, bottom)
    }

    private external fun nativeMeasure(handle: Long, widthSpec: Int, heightSpec: Int): Long

    private external fun nativeLayout(handle: Long, left: Int, top: Int, right: Int, bottom: Int)
}
