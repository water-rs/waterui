package dev.waterui.android.components

import androidx.compose.material3.Switch
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import dev.waterui.android.reactive.WuiBinding
import dev.waterui.android.runtime.NativeBindings
import dev.waterui.android.runtime.WuiAnyView
import dev.waterui.android.runtime.WuiTypeId
import dev.waterui.android.runtime.toTypeId

private val toggleTypeId: WuiTypeId by lazy { NativeBindings.waterui_toggle_id().toTypeId() }

private val toggleRenderer = WuiRenderer { node, env ->
    val struct = remember(node) { NativeBindings.waterui_force_as_toggle(node.rawPtr) }
    val binding = remember(struct.bindingPtr) {
        WuiBinding.bool(struct.bindingPtr, env).also { it.watch() }
    }
    val valueState = binding.state
    DisposableEffect(binding) {
        onDispose { binding.close() }
    }
    Switch(checked = valueState.value, onCheckedChange = { binding.set(it) })
    WuiAnyView(pointer = struct.labelPtr, environment = env)
}

internal fun MutableMap<WuiTypeId, WuiRenderer>.registerWuiToggle() {
    register({ toggleTypeId }, toggleRenderer)
}
