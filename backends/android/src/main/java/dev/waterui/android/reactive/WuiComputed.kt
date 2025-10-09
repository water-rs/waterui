package dev.waterui.android.reactive

import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.NativePointer
import dev.waterui.android.runtime.WatcherStruct
import dev.waterui.android.runtime.WuiEnvironment

/**
 * Read-only reactive wrapper that mirrors WaterUI computed signals.
 */
class WuiComputed<T>(
    computedPtr: Long,
    private val reader: (Long) -> T,
    private val watcherFactory: (Long, WatcherCallback<T>) -> WatcherStruct,
    private val watcherRegistrar: (Long, WatcherStruct) -> Long,
    private val dropper: (Long) -> Unit,
    private val env: WuiEnvironment
) : NativePointer(computedPtr) {

    private val stateDelegate = mutableStateOf(reader(computedPtr))
    private var watcherGuard: WatcherGuard? = null

    val state: State<T> get() = stateDelegate

    fun watch() {
        if (watcherGuard != null || isReleased) return
        val watcher = watcherFactory(raw()) { value, metadata ->
            stateDelegate.value = value
            // TODO: Handle animation metadata (metadata.animation)
        }
        watcherGuard = WatcherGuard(watcherRegistrar(raw(), watcher))
    }

    override fun close() {
        super.close()
        watcherGuard?.close()
        watcherGuard = null
    }

    override fun release(ptr: Long) {
        dropper(ptr)
    }

    companion object {
        fun double(ptr: Long, env: WuiEnvironment): WuiComputed<Double> =
            WuiComputed(
                computedPtr = ptr,
                reader = NativeBindings::waterui_read_binding_double, // TODO replace with computed-specific JNI call
                watcherFactory = { _, callback -> WatcherStructFactory.double(callback) },
                watcherRegistrar = NativeBindings::waterui_watch_binding_double,
                dropper = NativeBindings::waterui_drop_binding_double,
                env = env
            )
    }
}
