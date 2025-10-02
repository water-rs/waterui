package com.waterui.android.reactive

import com.sun.jna.Pointer
import com.waterui.android.ffi.CWaterUI
import java.io.Closeable

class WatcherGuard(private val inner: Pointer) : Closeable {
    override fun close() {
        CWaterUI.INSTANCE.waterui_drop_box_watcher_guard(inner)
    }
}
