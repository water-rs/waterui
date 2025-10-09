package dev.waterui.android.reactive

import androidx.compose.runtime.State
import androidx.compose.runtime.mutableStateOf
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.NativePointer
import dev.waterui.android.runtime.WatcherStruct
import dev.waterui.android.runtime.WuiEnvironment

/**
 * Generic binding wrapper translated from the Swift implementation. Uses Compose state for observation.
 */
class WuiBinding<T>(
    bindingPtr: Long,
    private val reader: (Long) -> T,
    private val writer: (Long, T) -> Unit,
    private val watcherFactory: (Long, WatcherCallback<T>) -> WatcherStruct,
    private val watcherRegistrar: (Long, WatcherStruct) -> Long,
    private val dropper: (Long) -> Unit,
    private val env: WuiEnvironment
) : NativePointer(bindingPtr) {

    private val stateDelegate = mutableStateOf(reader(bindingPtr))
    private var watcherGuard: WatcherGuard? = null
    private var syncingFromRust = false

    val state: State<T> get() = stateDelegate

    fun watch() {
        if (watcherGuard != null || isReleased) return
        val watcher = watcherFactory(raw()) { value, metadata ->
            syncingFromRust = true
            stateDelegate.value = value
            syncingFromRust = false
            // TODO: Handle animation metadata from metadata.animation
        }
        watcherGuard = WatcherGuard(watcherRegistrar(raw(), watcher))
    }

    fun set(value: T) {
        stateDelegate.value = value
        if (!syncingFromRust) {
            writer(raw(), value)
        }
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
        fun bool(bindingPtr: Long, env: WuiEnvironment): WuiBinding<Boolean> =
            WuiBinding(
                bindingPtr = bindingPtr,
                reader = NativeBindings::waterui_read_binding_bool,
                writer = { ptr, value -> NativeBindings.waterui_set_binding_bool(ptr, value) },
                watcherFactory = { _, callback -> WatcherStructFactory.bool(callback) },
                watcherRegistrar = NativeBindings::waterui_watch_binding_bool,
                dropper = NativeBindings::waterui_drop_binding_bool,
                env = env
            )

        fun int(bindingPtr: Long, env: WuiEnvironment): WuiBinding<Int> =
            WuiBinding(
                bindingPtr = bindingPtr,
                reader = NativeBindings::waterui_read_binding_int,
                writer = { ptr, value -> NativeBindings.waterui_set_binding_int(ptr, value) },
                watcherFactory = { _, callback -> WatcherStructFactory.int(callback) },
                watcherRegistrar = NativeBindings::waterui_watch_binding_int,
                dropper = NativeBindings::waterui_drop_binding_int,
                env = env
            )

        fun double(bindingPtr: Long, env: WuiEnvironment): WuiBinding<Double> =
            WuiBinding(
                bindingPtr = bindingPtr,
                reader = NativeBindings::waterui_read_binding_double,
                writer = { ptr, value -> NativeBindings.waterui_set_binding_double(ptr, value) },
                watcherFactory = { _, callback -> WatcherStructFactory.double(callback) },
                watcherRegistrar = NativeBindings::waterui_watch_binding_double,
                dropper = NativeBindings::waterui_drop_binding_double,
                env = env
            )

        fun str(bindingPtr: Long, env: WuiEnvironment): WuiBinding<String> =
            WuiBinding(
                bindingPtr = bindingPtr,
                reader = { ptr ->
                    val bytes = NativeBindings.waterui_read_binding_str(ptr)
                    bytes.decodeToString()
                },
                writer = { ptr, value ->
                    NativeBindings.waterui_set_binding_str(ptr, value.encodeToByteArray())
                },
                watcherFactory = { _, callback -> WatcherStructFactory.string(callback) },
                watcherRegistrar = NativeBindings::waterui_watch_binding_str,
                dropper = NativeBindings::waterui_drop_binding_str,
                env = env
            )
    }
}

/**
 * Produces placeholder watcher struct handles. Actual JNI trampoline implementation is pending.
 */
object WatcherStructFactory {
    fun bool(callback: WatcherCallback<Boolean>): WatcherStruct {
        // TODO: Build native watcher trampoline and return pointer trio.
        return WatcherStruct(0, 0, 0)
    }

    fun int(callback: WatcherCallback<Int>): WatcherStruct {
        return WatcherStruct(0, 0, 0)
    }

    fun double(callback: WatcherCallback<Double>): WatcherStruct {
        return WatcherStruct(0, 0, 0)
    }

    fun string(callback: WatcherCallback<String>): WatcherStruct {
        return WatcherStruct(0, 0, 0)
    }
}
