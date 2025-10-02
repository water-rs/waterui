package com.waterui.android.core

import com.sun.jna.Pointer
import com.waterui.android.ffi.CWaterUI

class WuiEnvironment(val inner: Pointer) {
    // The lifecycle of the native environment pointer is managed by the App's lifecycle.
    // When the app is destroyed, a corresponding call to waterui_drop_env should be made.
    // This class is a simple wrapper to hold the pointer.
}
