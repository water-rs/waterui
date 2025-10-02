package com.waterui.android.reactive

import com.sun.jna.Pointer
import com.waterui.android.ffi.CWaterUI
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.io.Closeable

// A generic wrapper for a binding value from Rust
abstract class Binding<T>(
    private val bindingPtr: Pointer,
    private val dropBinding: (Pointer?) -> Unit
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
    protected abstract fun writeValue(newValue: T)
    protected abstract fun startWatching()

    fun set(newValue: T) {
        writeValue(newValue)
    }

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
        dropBinding(bindingPtr)
    }
}

class BindingInt(private val ptr: Pointer) : Binding<Int>(ptr, CWaterUI.INSTANCE::waterui_drop_binding_int) {
    override fun readValue(): Int {
        return CWaterUI.INSTANCE.waterui_read_binding_int(ptr)
    }

    override fun writeValue(newValue: Int) {
        CWaterUI.INSTANCE.waterui_set_binding_int(ptr, newValue)
    }

    override fun startWatching() {
        val watcher = CWaterUI.WuiWatcher_i32().apply {
            this.call = object : CWaterUI.WuiWatcherIntCallback {
                override fun invoke(data: Pointer?, value: Int, metadata: Pointer?) {
                    updateValue(value)
                }
            }
            this.data = null
            this.drop = null
        }
        val guardPtr = CWaterUI.INSTANCE.waterui_watch_binding_int(ptr, watcher)
        setWatcher(guardPtr)
    }
}

class BindingDouble(private val ptr: Pointer) : Binding<Double>(ptr, CWaterUI.INSTANCE::waterui_drop_binding_double) {
    override fun readValue(): Double {
        return CWaterUI.INSTANCE.waterui_read_binding_double(ptr)
    }

    override fun writeValue(newValue: Double) {
        CWaterUI.INSTANCE.waterui_set_binding_double(ptr, newValue)
    }

    override fun startWatching() {
        val watcher = CWaterUI.WuiWatcher_f64().apply {
            this.call = object : CWaterUI.WuiWatcherDoubleCallback {
                override fun invoke(data: Pointer?, value: Double, metadata: Pointer?) {
                    updateValue(value)
                }
            }
            this.data = null
            this.drop = null
        }
        val guardPtr = CWaterUI.INSTANCE.waterui_watch_binding_double(ptr, watcher)
        setWatcher(guardPtr)
    }
}

class BindingStr(private val ptr: Pointer) : Binding<String>(ptr, CWaterUI.INSTANCE::waterui_drop_binding_str) {
    override fun readValue(): String {
        return CWaterUI.INSTANCE.waterui_read_binding_str(ptr).toKString()
    }

    override fun writeValue(newValue: String) {
        CWaterUI.INSTANCE.waterui_set_binding_str(ptr, CWaterUI.WuiStr.fromString(newValue))
    }

    override fun startWatching() {
        val watcher = CWaterUI.WuiWatcher_WuiStr().apply {
            this.call = object : CWaterUI.WuiWatcherStrCallback {
                override fun invoke(data: Pointer?, value: CWaterUI.WuiStr, metadata: Pointer?) {
                    updateValue(value.toKString())
                }
            }
            this.data = null
            this.drop = null
        }
        val guardPtr = CWaterUI.INSTANCE.waterui_watch_binding_str(ptr, watcher)
        setWatcher(guardPtr)
    }
}

class BindingBool(private val ptr: Pointer) : Binding<Boolean>(ptr, CWaterUI.INSTANCE::waterui_drop_binding_bool) {
    override fun readValue(): Boolean {
        return CWaterUI.INSTANCE.waterui_read_binding_bool(ptr)
    }

    override fun writeValue(newValue: Boolean) {
        CWaterUI.INSTANCE.waterui_set_binding_bool(ptr, newValue)
    }

    override fun startWatching() {
        val watcher = CWaterUI.WuiWatcher_bool().apply {
            this.call = object : CWaterUI.WuiWatcherBoolCallback {
                override fun invoke(data: Pointer?, value: Boolean, metadata: Pointer?) {
                    updateValue(value)
                }
            }
            this.data = null
            this.drop = null
        }
        val guardPtr = CWaterUI.INSTANCE.waterui_watch_binding_bool(ptr, watcher)
        setWatcher(guardPtr)
    }
}
