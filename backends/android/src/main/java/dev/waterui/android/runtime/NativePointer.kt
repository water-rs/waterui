package dev.waterui.android.runtime

import java.io.Closeable

/**
 * Canonical wrapper for native pointers obtained via JNI. Kotlin treats them as opaque [Long] values.
 * The pointer stays valid until [close] is invoked.
 */
abstract class NativePointer(
    protected var handle: Long
) : Closeable {

    /** Returns the raw native handle for JNI calls. */
    fun raw(): Long = handle

    /** Whether the pointer has already been released. */
    val isReleased: Boolean get() = handle == 0L

    override fun close() {
        if (!isReleased) {
            release(handle)
            handle = 0L
        }
    }

    /**
     * Release hook implemented by subclasses to drop native resources via JNI.
     * Implementations must tolerate repeated invocations with the same handle.
     */
    protected abstract fun release(ptr: Long)
}

/**
 * Helper that scopes a native pointer for the duration of [block] and releases it afterwards.
 */
inline fun <T : NativePointer, R> T.usePointer(block: (T) -> R): R {
    return try {
        block(this)
    } finally {
        close()
    }
}
