package com.waterui.android.components

import android.content.Context
import android.view.View
import com.google.android.material.button.MaterialButton
import com.sun.jna.Pointer
import com.waterui.android.WuiComponent
import com.waterui.android.core.WuiEnvironment
import com.waterui.android.ffi.CWaterUI

object WuiButton : WuiComponent {
    val id: CWaterUI.WuiTypeId by lazy { CWaterUI.INSTANCE.waterui_button_id() }

    override fun createView(anyView: Pointer, env: WuiEnvironment, context: Context): View {
        val buttonComponent = CWaterUI.INSTANCE.waterui_force_as_button(anyView)
        val actionPtr = buttonComponent.action
        val labelPtr = buttonComponent.label

        // The label of a button can be any view. We need to recursively create it.
        // However, a native Android button can only have a String as a label.
        // This is a mismatch between WaterUI's capabilities and the native backend.
        // For now, we will assume the label is a WuiText and extract its string content.
        // A more robust solution would be to create a custom view that can host any label view.

        val button = MaterialButton(context)

        // Process the label
        if (labelPtr != null) {
            val labelId = CWaterUI.INSTANCE.waterui_view_id(labelPtr)
            if (labelId == WuiText.id) {
                val textComponent = CWaterUI.INSTANCE.waterui_force_as_text(labelPtr)
                val computedStrPtr = textComponent.content
                val wuiStr = CWaterUI.INSTANCE.waterui_read_computed_str(computedStrPtr)
                button.text = wuiStr.toKString() // This consumes the WuiStr
                CWaterUI.INSTANCE.waterui_drop_computed_str(computedStrPtr)
            } else {
                // If the label is not text, we can't display it on a standard button.
                // Set a placeholder text.
                button.text = "Button"
                // We still need to drop the label view to avoid memory leaks.
                CWaterUI.INSTANCE.waterui_drop_any_view(labelPtr)
            }
        }

        // Set the click listener
        button.setOnClickListener {
            if (actionPtr != null) {
                CWaterUI.INSTANCE.waterui_call_action(actionPtr, env.inner)
            }
        }

        // We don't drop the action pointer here because the button might be clicked multiple times.
        // It should be dropped when the button view is destroyed.
        // TODO: Implement proper lifecycle management for the action pointer.

        return button
    }
}
