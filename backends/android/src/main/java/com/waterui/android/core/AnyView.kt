package com.waterui.android.core

import android.content.Context
import android.view.View
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.components.WuiText
import com.waterui.android.ffi.CWaterUI

/**
 * Creates a native Android View from a Rust AnyView pointer.
 *
 * This function is the core of the rendering engine. It checks the type ID of the
 * view and dispatches to the appropriate component creator. If the type is not
 * a known component, it assumes it's a modifier and asks for its body.
 */
fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
    val id = CWaterUI.INSTANCE.waterui_view_id(anyView)
    val componentCreator = Render.map[id]

    return if (componentCreator != null) {
        // The pointer is a known component, so render it.
        componentCreator.createView(anyView, env, context)
    } else {
        // The pointer is a modifier or a view whose body we need to render.
        // Get the body and recursively call createView.
        val next = CWaterUI.INSTANCE.waterui_view_body(anyView, env.inner)
        createView(next, env, context)
    }
}

/**
 * A registry of all known WaterUI components for the Android backend.
 */
private object Render {
    val map: Map<CWaterUI.WuiTypeId, WuiComponent>

    init {
        val components = listOf(
            WuiText,
            WuiButton,
            WuiContainer,
            WuiSpacer,
            WuiScrollView,
            WuiDivider,
            WuiProgress,
            WuiStepper,
            WuiSlider,
            WuiTextField,
            WuiToggle
        )
        map = components.associateBy { it.id }
    }
}
