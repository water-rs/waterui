package dev.waterui.android.reactive

import androidx.compose.runtime.mutableStateOf
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.NativePointer
import dev.waterui.android.runtime.WuiEnvironment

/**
 * Holder around native watcher metadata. Manages drop semantics.
 */
class WatcherGuard(pointer: Long) : NativePointer(pointer) {
    override fun release(ptr: Long) {
        NativeBindings.waterui_drop_watcher_guard(ptr)
    }
}

data class WuiWatcherMetadata(val pointer: Long) {
    val animation: Int get() = NativeBindings.waterui_get_animation(pointer)
    // TODO: Map to Compose animation specs.
}

/**
 * Convenience container that carries mutable state and ensures we dispose watchers when recomposed.
 */
class ObservableValue<T>(initial: T) {
    var state = mutableStateOf(initial)
        private set

    fun update(value: T, metadata: WuiWatcherMetadata) {
        // TODO: Bridge animation metadata.
        state.value = value
    }
}

fun interface WatcherCallback<T> {
    fun onChanged(value: T, metadata: WuiWatcherMetadata)
}
