package dev.waterui.android.components

import androidx.compose.material3.Button
import androidx.compose.runtime.remember
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiAnyView
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val buttonTypeId: WuiTypeId by lazy { NativeBindings.waterui_button_id().toTypeId() }

private val buttonRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_button(node.rawPtr) }
    Button(onClick = {
        // TODO: Provide proper env cloning / pointer safety.
        NativeBindings.waterui_call_action(struct.actionPtr, env.raw())
    }) {
        WuiAnyView(pointer = struct.labelPtr, environment = env)
    }
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiButton() {
    register({ buttonTypeId }, buttonRenderer)
}
