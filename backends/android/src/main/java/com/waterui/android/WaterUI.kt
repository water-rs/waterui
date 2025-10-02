package com.waterui.android

import android.content.Context
import android.view.View
import com.sun.jna.Pointer
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.core.createView
import com.waterui.android.ffi.CWaterUI

/**
 * The main entry point for a WaterUI application on Android.
 *
 * This function initializes the Rust environment and returns the root View
 * that should be set as the content view of an Activity.
 */
fun App(context: Context): View {
    val env = WuiEnvironment(CWaterUI.INSTANCE.waterui_init())
    val rootView = CWaterUI.INSTANCE.waterui_main()
    return createView(rootView, env, context)
}

/**
 * A component that can be rendered from a Rust AnyView pointer.
 */
interface WuiComponent {
    fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View
}
