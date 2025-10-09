package dev.waterui.android.components

import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiAnyView
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val stepperTypeId: WuiTypeId by lazy { NativeBindings.waterui_stepper_id().toTypeId() }

private val stepperRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_stepper(node.rawPtr) }
    val value = remember { mutableStateOf(0) }
    // TODO: Link to bindings/computed for stepper configuration.
    TextButton(onClick = { value.value += 1 }) {
        WuiAnyView(pointer = struct.labelPtr, environment = env)
        Text(" ${'$'}{value.value}")
    }
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiStepper() {
    register({ stepperTypeId }, stepperRenderer)
}
