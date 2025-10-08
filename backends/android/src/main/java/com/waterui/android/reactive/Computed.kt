package com.waterui.android.reactive

import com.sun.jna.Pointer
import com.waterui.android.ffi.CWaterUI
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.io.Closeable

// A generic wrapper for a computed value from Rust
abstract class Computed<T>(
    private val computedPtr: Pointer,
    private val dropComputed: (Pointer?) -> Unit
) : Closeable {
    protected val _value: MutableStateFlow<T>
    val value get() = _value.asStateFlow()

    private var watcherGuard: WatcherGuard? = null

    init {
        _value = MutableStateFlow(readValue())
        startWatching()
    }

    // Abstract methods for subclasses to implement
    protected abstract fun readValue(): T
    protected abstract fun startWatching()

    protected fun setWatcher(guard: Pointer) {
        watcherGuard = WatcherGuard(guard)
    }

    protected fun updateValue(newValue: T) {
        MainScope().launch {
            _value.emit(newValue)
        }
    }

    override fun close() {
        watcherGuard?.close()
        dropComputed(computedPtr)
    }
}

class ComputedStr(private val ptr: Pointer) : Computed<String>(ptr, CWaterUI.INSTANCE::waterui_drop_computed_str) {
    override fun readValue(): String {
        return CWaterUI.INSTANCE.waterui_read_computed_str(ptr).toKString()
    }

    override fun startWatching() {
        val watcher = CWaterUI.WuiWatcher_WuiStr().apply {
            // The callback that JNA will invoke
            this.call = object : CWaterUI.WuiWatcherStrCallback {
                override fun invoke(data: Pointer?, value: CWaterUI.WuiStr, metadata: Pointer?) {
                    updateValue(value.toKString())
                }
            }
            // No external data needed for the callback
            this.data = null
            this.drop = null
        }
        val guardPtr = CWaterUI.INSTANCE.waterui_watch_computed_str(ptr, watcher)
        setWatcher(guardPtr)
    }
}

class ComputedAttributedStr(private val ptr: Pointer) : Computed<String>(ptr, CWaterUI.INSTANCE::waterui_drop_computed_attributed_str) {
    override fun readValue(): String {
        return CWaterUI.INSTANCE.waterui_read_computed_attributed_str(ptr).toPlainString()
    }

    override fun startWatching() {
        val watcher = CWaterUI.WuiWatcher_WuiAttributedStr().apply {
            this.call = object : CWaterUI.WuiWatcherAttributedStrCallback {
                override fun invoke(data: Pointer?, value: CWaterUI.WuiAttributedStr, metadata: Pointer?) {
                    updateValue(value.toPlainString())
                }
            }
            this.data = null
            this.drop = null
        }
        val guardPtr = CWaterUI.INSTANCE.waterui_watch_computed_attributed_str(ptr, watcher)
        setWatcher(guardPtr)
    }
}

class ComputedDouble(private val ptr: Pointer) : Computed<Double>(ptr, CWaterUI.INSTANCE::waterui_drop_computed_double) {
    override fun readValue(): Double {
        return CWaterUI.INSTANCE.waterui_read_computed_double(ptr)
    }

    override fun startWatching() {
        val watcher = CWaterUI.WuiWatcher_f32().apply {
            this.call = object : CWaterUI.WuiWatcherDoubleCallback {
                override fun invoke(data: Pointer?, value: Double, metadata: Pointer?) {
                    updateValue(value)
                }
            }
            this.data = null
            this.drop = null
        }
        val guardPtr = CWaterUI.INSTANCE.waterui_watch_computed_double(ptr, watcher)
        setWatcher(guardPtr)
    }
}
